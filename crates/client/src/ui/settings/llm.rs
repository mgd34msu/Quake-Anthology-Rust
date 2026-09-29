//! LLM settings menu.
//!
//! Donor provenance: `src/ui/settings/llm.ts` in full. The donor service
//! (`src/llm/settings.ts`) is async; this port is sync (`Result<(), String>`)
//! so native menus stay single-threaded. The donor `AbortController`
//! discovery handle is dropped; the `generation` counter is kept to invalidate
//! stale completions across cancel and close.

use std::cell::RefCell;
use std::rc::Rc;

use crate::ui::common::controller::NativeUiController;
use crate::ui::common::layout::{menu_row, MenuRowOptions};
use crate::ui::types::{SeatUiController as _, UiChoice, UiControl, UiControlId, UiControlKind, UiMenu, UiMenuId};

/// LLM provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LlmProvider {
    /// ChatGPT subscription.
    ChatGptSubscription,
    /// ChatGPT API.
    ChatGptApi,
    /// Other API.
    OtherApi,
}

impl LlmProvider {
    /// Donor provider id.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            LlmProvider::ChatGptSubscription => "chatgpt-subscription",
            LlmProvider::ChatGptApi => "chatgpt-api",
            LlmProvider::OtherApi => "other-api",
        }
    }

    /// Parse a donor provider id.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "chatgpt-subscription" => Some(LlmProvider::ChatGptSubscription),
            "chatgpt-api" => Some(LlmProvider::ChatGptApi),
            "other-api" => Some(LlmProvider::OtherApi),
            _ => None,
        }
    }

    /// Every provider in menu order.
    #[must_use]
    pub fn all() -> &'static [LlmProvider] {
        &[
            LlmProvider::ChatGptSubscription,
            LlmProvider::ChatGptApi,
            LlmProvider::OtherApi,
        ]
    }
}

/// UI-local provider state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmProviderState {
    /// Whether a credential is stored.
    pub configured: bool,
    /// Stored model id (empty when unset).
    pub model: String,
    /// Base URL (other-api only; empty otherwise).
    pub base_url: String,
}

/// Provider states keyed by donor provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmProviders {
    /// ChatGPT subscription state.
    pub subscription: LlmProviderState,
    /// ChatGPT API state.
    pub api: LlmProviderState,
    /// Other API state.
    pub other: LlmProviderState,
}

impl LlmProviders {
    /// Borrow one provider's state.
    #[must_use]
    pub fn get(&self, provider: LlmProvider) -> &LlmProviderState {
        match provider {
            LlmProvider::ChatGptSubscription => &self.subscription,
            LlmProvider::ChatGptApi => &self.api,
            LlmProvider::OtherApi => &self.other,
        }
    }
}

/// UI-local model entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmModel {
    /// Model id.
    pub id: String,
    /// Supported reasoning efforts.
    pub reasoning_efforts: Vec<String>,
}

/// UI-local catalog status.
///
/// The donor has `idle`/`loading`/`ready`/`error`; `Idle` covers donor `idle`
/// and `Ready` covers donor `ready` so the save-flow refresh check
/// (`idle` only) stays exact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmCatalogStatus {
    /// Idle (donor `idle`).
    Idle,
    /// Loading.
    Loading,
    /// Ready (donor `ready`).
    Ready,
    /// Load failed.
    Error {
        /// Failure message.
        message: String,
    },
}

/// UI-local model catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmCatalog {
    /// Loaded models.
    pub models: Vec<LlmModel>,
    /// Catalog status.
    pub status: LlmCatalogStatus,
}

/// Catalogs keyed by provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmCatalogs {
    /// ChatGPT subscription catalog.
    pub subscription: LlmCatalog,
    /// ChatGPT API catalog.
    pub api: LlmCatalog,
    /// Other API catalog.
    pub other: LlmCatalog,
}

impl LlmCatalogs {
    /// Borrow one provider's catalog.
    #[must_use]
    pub fn get(&self, provider: LlmProvider) -> &LlmCatalog {
        match provider {
            LlmProvider::ChatGptSubscription => &self.subscription,
            LlmProvider::ChatGptApi => &self.api,
            LlmProvider::OtherApi => &self.other,
        }
    }
}

/// Subscription sign-in state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmSubscriptionAuth {
    /// Idle.
    Idle,
    /// Waiting for browser sign-in.
    Pending,
    /// Sign-in failed.
    Error {
        /// Failure message.
        message: String,
    },
}

/// Settings load error (the menu shows the message only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmError {
    /// Failure message.
    pub message: String,
}

/// UI-local snapshot mirroring `LlmSettingsSnapshot`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmSnapshot {
    /// Selected provider.
    pub provider: LlmProvider,
    /// Provider states.
    pub providers: LlmProviders,
    /// Model catalogs.
    pub catalogs: LlmCatalogs,
    /// Subscription sign-in state.
    pub subscription_auth: LlmSubscriptionAuth,
    /// Reasoning effort for the selected provider (`None` is Model default).
    pub reasoning_effort: Option<String>,
    /// Load errors (the menu shows the first message).
    pub errors: Vec<LlmError>,
}

/// Sync UI surface over LLM settings.
///
/// The donor `LlmSettingsService` methods are async (`Promise`); this trait is
/// sync so menu callbacks stay single-threaded. `Err` carries the message the
/// menu shows. `refresh_models` drops the donor `AbortSignal`: it updates the
/// snapshot synchronously and the caller re-reads via [`LlmSettingsUi::read`].
pub trait LlmSettingsUi {
    /// Read the current snapshot.
    fn read(&self) -> LlmSnapshot;
    /// Select the active provider (donor `selectProvider`).
    fn select_provider(&mut self, provider: LlmProvider) -> Result<(), String>;
    /// Store a model id (donor `setModel`).
    fn set_model(&mut self, provider: LlmProvider, model: &str) -> Result<(), String>;
    /// Store an API key (donor `saveApiKey`).
    fn save_api_key(&mut self, provider: LlmProvider, key: &str) -> Result<(), String>;
    /// Remove a stored credential (donor `removeCredential`).
    fn remove_credential(&mut self, provider: LlmProvider) -> Result<(), String>;
    /// Store the other-API base URL plus model (donor `saveOtherService`;
    /// transport is always OpenAI Chat Completions).
    fn save_other_service(&mut self, base_url: &str, model: &str) -> Result<(), String>;
    /// Start browser sign-in (donor `signInSubscription`).
    fn sign_in_subscription(&mut self) -> Result<(), String>;
    /// Cancel a pending sign-in (donor `cancelSignIn`).
    fn cancel_sign_in(&mut self);
    /// Reload one provider's models (donor `refreshModels` without the signal).
    fn refresh_models(&mut self, provider: LlmProvider) -> Result<(), String>;
    /// Store a reasoning effort (`None` is Model default; donor
    /// `setReasoningEffort`).
    fn set_reasoning_effort(&mut self, provider: LlmProvider, effort: Option<&str>) -> Result<(), String>;
}

/// Registered LLM menus: the root plus the models picker.
pub struct LlmMenus {
    /// Root menu id (`menu:settings:llm`).
    pub root: UiMenuId,
    controller: Rc<RefCell<NativeUiController>>,
    service: Rc<RefCell<dyn LlmSettingsUi>>,
    state: Rc<RefCell<LlmUiState>>,
    models: UiMenuId,
}

impl LlmMenus {
    /// Unregister both menus, cancel sign-in, and clear drafts (donor `dispose`).
    pub fn dispose(self) {
        self.controller.borrow_mut().unregister(&self.models);
        self.controller.borrow_mut().unregister(&self.root);
        self.service.borrow_mut().cancel_sign_in();
        self.state.borrow_mut().clear();
    }
}

impl std::fmt::Debug for LlmMenus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmMenus")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

/// Drafts plus busy/status/generation/page (donor closure state).
///
/// `effort` mirrors donor `string | null | undefined`: `None` is untouched,
/// `Some(None)` is Model default, `Some(Some(_))` is a named effort.
#[derive(Debug, Clone)]
struct LlmUiState {
    key: String,
    model: Option<String>,
    base_url: Option<String>,
    busy: bool,
    status: String,
    generation: u64,
    effort: Option<Option<String>>,
    page: usize,
}

impl LlmUiState {
    fn new() -> Self {
        Self {
            key: String::new(),
            model: None,
            base_url: None,
            busy: false,
            status: String::new(),
            generation: 0,
            effort: None,
            page: 0,
        }
    }

    /// Clear drafts only (donor `clear`).
    fn clear(&mut self) {
        self.key.clear();
        self.model = None;
        self.base_url = None;
        self.effort = None;
    }
}

/// Build a control id from a static template; the templates below always carry
/// the `ui:` namespace and a name part, so a failure is a programming bug.
fn control_id(text: &str) -> UiControlId {
    match UiControlId::new(text) {
        Ok(id) => id,
        Err(_) => panic!("static UI control id is invalid: {text}"),
    }
}

/// Build a menu id from a static template; the templates below always carry
/// the `menu:` namespace and a scope part, so a failure is a programming bug.
fn menu_id(text: &str) -> UiMenuId {
    match UiMenuId::new(text) {
        Ok(id) => id,
        Err(_) => panic!("static UI menu id is invalid: {text}"),
    }
}

/// Wrap a status message like donor `message.match(/.{1,54}(?:\s|$)|.{1,54}/g)`.
///
/// Up to 54 chars per line, breaking before trailing whitespace when possible;
/// over-long words split mid-word. Empty input yields no lines.
fn wrap_message(message: &str) -> Vec<String> {
    if message.is_empty() {
        return Vec::new();
    }
    let normalized = message.replace(['\r', '\n', '\u{2028}', '\u{2029}'], " ");
    let chars: Vec<char> = normalized.chars().collect();
    let total = chars.len();
    let mut lines = Vec::new();
    let mut start = 0;
    while start < total {
        let take = (total - start).min(54);
        let mut matched: Option<(usize, usize)> = None;
        for width in (1..=take).rev() {
            if start + width == total {
                matched = Some((start, start + width));
                break;
            }
            if start + width < total && chars[start + width].is_whitespace() {
                matched = Some((start, start + width + 1));
                break;
            }
        }
        if let Some((from, to)) = matched {
            lines.push(chars[from..to].iter().collect());
            start = to;
        } else {
            lines.push(chars[start..start + take].iter().collect());
            start += take;
        }
    }
    lines
}

/// Reload the selected provider's models when configured (donor `refresh`
/// without the `AbortController`); failures are ignored for a later retry.
fn refresh_if_configured(service: &Rc<RefCell<dyn LlmSettingsUi>>) {
    let snapshot = service.borrow().read();
    if !snapshot.providers.get(snapshot.provider).configured {
        return;
    }
    let _ = service.borrow_mut().refresh_models(snapshot.provider);
}

/// Run one sync operation with donor `run` busy/status/generation handling.
///
/// `success` runs only when the generation is unchanged; on error the message
/// becomes the status. The generation can only change across cancel or close,
/// which cannot interleave a sync call, but the check is kept for parity.
fn run_operation(
    state: &Rc<RefCell<LlmUiState>>,
    operation: impl FnOnce() -> Result<(), String>,
    success: impl FnOnce(),
) {
    let current = {
        let mut ui = state.borrow_mut();
        if ui.busy {
            return;
        }
        let current = ui.generation;
        ui.busy = true;
        ui.status = "Working...".to_string();
        current
    };
    match operation() {
        Ok(()) => {
            if state.borrow().generation == current {
                success();
            }
            if state.borrow().generation == current {
                state.borrow_mut().busy = false;
            }
        }
        Err(message) => {
            if state.borrow().generation == current {
                let mut ui = state.borrow_mut();
                if ui.generation == current {
                    ui.status = message;
                    ui.busy = false;
                }
            }
        }
    }
}

/// Make a disabled status button (donor `button` with `enabled = false`).
fn status_button(id: &str, label: String, row: i32) -> UiControl {
    UiControl {
        id: control_id(id),
        label,
        rect: menu_row(row, &MenuRowOptions::default()),
        enabled: false,
        visible: true,
        kind: UiControlKind::Button {
            on_activate: Rc::new(|_| {}),
        },
    }
}

/// Build the root menu (donor `controller.register(root, ...)` factory).
fn root_menu(
    controller: &Rc<RefCell<NativeUiController>>,
    service: &Rc<RefCell<dyn LlmSettingsUi>>,
    state: &Rc<RefCell<LlmUiState>>,
) -> UiMenu {
    let snapshot = service.borrow().read();
    let provider = snapshot.provider;
    let selected = snapshot.providers.get(provider).clone();
    let pending = matches!(snapshot.subscription_auth, LlmSubscriptionAuth::Pending);
    let catalog = snapshot.catalogs.get(provider).clone();
    let (chosen_model, busy, status, effort_draft, key_draft, base_url_draft) = {
        let ui = state.borrow();
        (
            ui.model.clone().unwrap_or_else(|| selected.model.clone()),
            ui.busy,
            ui.status.clone(),
            ui.effort.clone(),
            ui.key.clone(),
            ui.base_url.clone(),
        )
    };
    let metadata = catalog.models.iter().find(|item| item.id == chosen_model);

    let mut controls: Vec<UiControl> = Vec::new();

    {
        let state_select = Rc::clone(state);
        let service_select = Rc::clone(service);
        controls.push(UiControl {
            id: control_id("ui:llm:provider"),
            label: "Provider".to_string(),
            rect: menu_row(0, &MenuRowOptions::default()),
            enabled: !pending,
            visible: true,
            kind: UiControlKind::Choice {
                choices: vec![
                    UiChoice {
                        id: "chatgpt-subscription".to_string(),
                        label: "ChatGPT Subscription".to_string(),
                    },
                    UiChoice {
                        id: "chatgpt-api".to_string(),
                        label: "ChatGPT API".to_string(),
                    },
                    UiChoice {
                        id: "other-api".to_string(),
                        label: "Other API".to_string(),
                    },
                ],
                selected: Some(provider.as_str().to_string()),
                on_select: Rc::new(move |_, value: &str| {
                    if state_select.borrow().busy {
                        return;
                    }
                    let Some(next) = LlmProvider::parse(value) else {
                        return;
                    };
                    state_select.borrow_mut().clear();
                    let service_op = Rc::clone(&service_select);
                    let state_ok = Rc::clone(&state_select);
                    let service_refresh = Rc::clone(&service_select);
                    run_operation(
                        &state_select,
                        move || service_op.borrow_mut().select_provider(next),
                        move || {
                            state_ok.borrow_mut().status.clear();
                            refresh_if_configured(&service_refresh);
                        },
                    );
                }),
            },
        });
    }

    let status_label = if pending {
        "Waiting for browser sign-in".to_string()
    } else if selected.configured {
        "Credential stored".to_string()
    } else {
        "No credential stored".to_string()
    };
    controls.push(status_button("ui:llm:status", status_label, 1));

    {
        let controller_open = Rc::clone(controller);
        let state_page = Rc::clone(state);
        let model_label = if chosen_model.is_empty() {
            "Select a model".to_string()
        } else {
            chosen_model.clone()
        };
        controls.push(UiControl {
            id: control_id("ui:llm:model"),
            label: format!("Model: {model_label}"),
            rect: menu_row(2, &MenuRowOptions::default()),
            enabled: !busy,
            visible: true,
            kind: UiControlKind::Button {
                on_activate: Rc::new(move |_| {
                    state_page.borrow_mut().page = 0;
                    let models = menu_id("menu:settings:llm-models");
                    let _ = controller_open.borrow_mut().open_menu(&models);
                }),
            },
        });
    }

    {
        let effective: Option<String> = match effort_draft {
            None => snapshot.reasoning_effort.clone(),
            Some(draft) => draft,
        };
        let mut choices = vec![UiChoice {
            id: String::new(),
            label: "Model default".to_string(),
        }];
        if let Some(meta) = metadata {
            for effort in &meta.reasoning_efforts {
                choices.push(UiChoice {
                    id: effort.clone(),
                    label: effort.clone(),
                });
            }
        }
        let enabled = !busy && metadata.is_some_and(|meta| !meta.reasoning_efforts.is_empty());
        let state_effort = Rc::clone(state);
        controls.push(UiControl {
            id: control_id("ui:llm:effort"),
            label: "Reasoning effort".to_string(),
            rect: menu_row(3, &MenuRowOptions::default()),
            enabled,
            visible: true,
            kind: UiControlKind::Choice {
                choices,
                selected: Some(effective.unwrap_or_default()),
                on_select: Rc::new(move |_, value: &str| {
                    state_effort.borrow_mut().effort = Some(if value.is_empty() {
                        None
                    } else {
                        Some(value.to_string())
                    });
                }),
            },
        });
    }

    if provider == LlmProvider::ChatGptSubscription {
        {
            let state_signin = Rc::clone(state);
            let service_signin = Rc::clone(service);
            let selected_model = selected.model.clone();
            controls.push(UiControl {
                id: control_id("ui:llm:signin"),
                label: "Sign in with ChatGPT".to_string(),
                rect: menu_row(4, &MenuRowOptions::default()),
                enabled: !busy && !pending,
                visible: true,
                kind: UiControlKind::Button {
                    on_activate: Rc::new(move |_| {
                        let service_op = Rc::clone(&service_signin);
                        let state_ok = Rc::clone(&state_signin);
                        let service_refresh = Rc::clone(&service_signin);
                        let selected_model = selected_model.clone();
                        run_operation(
                            &state_signin,
                            move || service_op.borrow_mut().sign_in_subscription(),
                            move || {
                                let mut ui = state_ok.borrow_mut();
                                ui.key.clear();
                                let draft = ui.model.clone();
                                let base = draft.clone().unwrap_or_else(|| selected_model.clone());
                                ui.status = if base.trim().is_empty() {
                                    "Signed in. Select a model, then save settings.".to_string()
                                } else if draft.is_some() {
                                    "Signed in. Save settings to use this model.".to_string()
                                } else {
                                    "Signed in.".to_string()
                                };
                                drop(ui);
                                refresh_if_configured(&service_refresh);
                            },
                        );
                    }),
                },
            });
        }
        {
            let state_cancel = Rc::clone(state);
            let service_cancel = Rc::clone(service);
            controls.push(UiControl {
                id: control_id("ui:llm:cancel-signin"),
                label: "Cancel sign-in".to_string(),
                rect: menu_row(5, &MenuRowOptions::default()),
                enabled: pending,
                visible: true,
                kind: UiControlKind::Button {
                    on_activate: Rc::new(move |_| {
                        state_cancel.borrow_mut().generation += 1;
                        service_cancel.borrow_mut().cancel_sign_in();
                        let mut ui = state_cancel.borrow_mut();
                        ui.busy = false;
                        ui.clear();
                        ui.status = "Sign-in canceled".to_string();
                    }),
                },
            });
        }
        {
            let state_signout = Rc::clone(state);
            let service_signout = Rc::clone(service);
            controls.push(UiControl {
                id: control_id("ui:llm:signout"),
                label: "Sign out".to_string(),
                rect: menu_row(6, &MenuRowOptions::default()),
                enabled: !busy && selected.configured,
                visible: true,
                kind: UiControlKind::Button {
                    on_activate: Rc::new(move |_| {
                        let service_op = Rc::clone(&service_signout);
                        let state_ok = Rc::clone(&state_signout);
                        run_operation(
                            &state_signout,
                            move || service_op.borrow_mut().remove_credential(provider),
                            move || {
                                let mut ui = state_ok.borrow_mut();
                                ui.clear();
                                ui.status = "Saved".to_string();
                            },
                        );
                    }),
                },
            });
        }
    } else {
        {
            let state_key = Rc::clone(state);
            controls.push(UiControl {
                id: control_id("ui:llm:key"),
                label: "Paste API key".to_string(),
                rect: menu_row(4, &MenuRowOptions::default()),
                enabled: !busy,
                visible: true,
                kind: UiControlKind::TextEntry {
                    masked: true,
                    text: key_draft,
                    maximum_length: 4096,
                    on_change: Rc::new(move |_, value: &str| {
                        state_key.borrow_mut().key = value.to_string();
                    }),
                    on_submit: Rc::new(|_, _| {}),
                },
            });
        }
        {
            let state_remove = Rc::clone(state);
            let service_remove = Rc::clone(service);
            controls.push(UiControl {
                id: control_id("ui:llm:remove-key"),
                label: "Remove stored key".to_string(),
                rect: menu_row(5, &MenuRowOptions::default()),
                enabled: !busy && selected.configured,
                visible: true,
                kind: UiControlKind::Button {
                    on_activate: Rc::new(move |_| {
                        let service_op = Rc::clone(&service_remove);
                        let state_ok = Rc::clone(&state_remove);
                        run_operation(
                            &state_remove,
                            move || service_op.borrow_mut().remove_credential(provider),
                            move || {
                                let mut ui = state_ok.borrow_mut();
                                ui.clear();
                                ui.status = "Saved".to_string();
                            },
                        );
                    }),
                },
            });
        }
        if provider == LlmProvider::OtherApi {
            {
                let state_base = Rc::clone(state);
                controls.push(UiControl {
                    id: control_id("ui:llm:base-url"),
                    label: "Base URL".to_string(),
                    rect: menu_row(6, &MenuRowOptions::default()),
                    enabled: !busy,
                    visible: true,
                    kind: UiControlKind::TextEntry {
                        masked: false,
                        text: base_url_draft.unwrap_or_else(|| snapshot.providers.other.base_url.clone()),
                        maximum_length: 512,
                        on_change: Rc::new(move |_, value: &str| {
                            state_base.borrow_mut().base_url = Some(value.to_string());
                        }),
                        on_submit: Rc::new(|_, _| {}),
                    },
                });
            }
            controls.push(status_button(
                "ui:llm:transport",
                "OpenAI-compatible Chat Completions".to_string(),
                7,
            ));
        }
    }

    let message = if busy {
        status.clone()
    } else if matches!(catalog.status, LlmCatalogStatus::Loading) {
        "Loading models... Settings can still be saved.".to_string()
    } else if let LlmCatalogStatus::Error { message } = &catalog.status {
        message.clone()
    } else if !status.is_empty() {
        status.clone()
    } else if let LlmSubscriptionAuth::Error { message } = &snapshot.subscription_auth {
        message.clone()
    } else if let Some(first) = snapshot.errors.first() {
        first.message.clone()
    } else if chosen_model.is_empty() {
        "Select a model after signing in or saving an API key.".to_string()
    } else {
        "Console: llm_ask or llm_exec".to_string()
    };
    let lines = wrap_message(&message);

    {
        let state_save = Rc::clone(state);
        let service_save = Rc::clone(service);
        let selected_model = selected.model.clone();
        let other_base = snapshot.providers.other.base_url.clone();
        let catalog_was_idle = matches!(catalog.status, LlmCatalogStatus::Idle);
        controls.push(UiControl {
            id: control_id("ui:llm:save"),
            label: "Save settings".to_string(),
            rect: menu_row(
                8,
                &MenuRowOptions {
                    width: Some(248.0),
                    ..MenuRowOptions::default()
                },
            ),
            enabled: !busy && !pending,
            visible: true,
            kind: UiControlKind::Button {
                on_activate: Rc::new(move |_| {
                    let (draft_effort, draft_key, draft_model, draft_base) = {
                        let ui = state_save.borrow();
                        (
                            ui.effort.clone(),
                            ui.key.clone(),
                            ui.model.clone().unwrap_or_else(|| selected_model.clone()),
                            ui.base_url.clone().unwrap_or_else(|| other_base.clone()),
                        )
                    };
                    state_save.borrow_mut().key.clear();
                    let service_op = Rc::clone(&service_save);
                    let state_ok = Rc::clone(&state_save);
                    let service_refresh = Rc::clone(&service_save);
                    let op_effort = draft_effort;
                    let op_key = draft_key.clone();
                    let op_model = draft_model.clone();
                    let op_base = draft_base;
                    let ok_key = draft_key;
                    let ok_model = draft_model;
                    run_operation(
                        &state_save,
                        move || {
                            if provider == LlmProvider::OtherApi {
                                service_op.borrow_mut().save_other_service(&op_base, &op_model)?;
                            } else if !op_model.is_empty() {
                                service_op.borrow_mut().set_model(provider, &op_model)?;
                            }
                            if let Some(effort) = &op_effort {
                                service_op
                                    .borrow_mut()
                                    .set_reasoning_effort(provider, effort.as_deref())?;
                            }
                            if provider != LlmProvider::ChatGptSubscription && !op_key.is_empty() {
                                service_op.borrow_mut().save_api_key(provider, &op_key)?;
                            }
                            Ok(())
                        },
                        move || {
                            let changed_connection = state_ok.borrow().base_url.is_some();
                            {
                                let mut ui = state_ok.borrow_mut();
                                ui.clear();
                                ui.status = if ok_model.is_empty() {
                                    "Saved. Select a model to use console requests.".to_string()
                                } else {
                                    "Saved".to_string()
                                };
                            }
                            if !ok_key.is_empty() || changed_connection || catalog_was_idle {
                                refresh_if_configured(&service_refresh);
                            }
                        },
                    );
                }),
            },
        });
    }

    {
        let service_refresh = Rc::clone(service);
        controls.push(UiControl {
            id: control_id("ui:llm:refresh"),
            label: "Refresh models".to_string(),
            rect: menu_row(
                8,
                &MenuRowOptions {
                    x: Some(328.0),
                    width: Some(248.0),
                    ..MenuRowOptions::default()
                },
            ),
            enabled: !busy && !pending && selected.configured,
            visible: true,
            kind: UiControlKind::Button {
                on_activate: Rc::new(move |_| {
                    refresh_if_configured(&service_refresh);
                }),
            },
        });
    }

    for (index, line) in lines.iter().take(2).enumerate() {
        controls.push(status_button(
            &format!("ui:llm:message:{index}"),
            line.trim().to_string(),
            9 + index as i32,
        ));
    }

    {
        let controller_back = Rc::clone(controller);
        controls.push(UiControl {
            id: control_id("ui:llm:back"),
            label: "Back".to_string(),
            rect: menu_row(11, &MenuRowOptions::default()),
            enabled: true,
            visible: true,
            kind: UiControlKind::Button {
                on_activate: Rc::new(move |_| {
                    controller_back.borrow_mut().close_menu();
                }),
            },
        });
    }

    let state_open = Rc::clone(state);
    let service_open = Rc::clone(service);
    let state_close = Rc::clone(state);
    let service_close = Rc::clone(service);
    UiMenu {
        scroll: None,
        id: menu_id("menu:settings:llm"),
        title: "LLM options".to_string(),
        full_screen: false,
        controls,
        on_open: Rc::new(move |_| {
            {
                let mut ui = state_open.borrow_mut();
                ui.clear();
                ui.status.clear();
            }
            refresh_if_configured(&service_open);
        }),
        on_close: Rc::new(move |_| {
            {
                let mut ui = state_close.borrow_mut();
                ui.generation += 1;
                ui.clear();
                ui.busy = false;
                ui.status.clear();
            }
            service_close.borrow_mut().cancel_sign_in();
        }),
    }
}

/// Build the models picker (donor `menu:settings:llm-models` factory).
fn models_menu(
    controller: &Rc<RefCell<NativeUiController>>,
    service: &Rc<RefCell<dyn LlmSettingsUi>>,
    state: &Rc<RefCell<LlmUiState>>,
) -> UiMenu {
    let snapshot = service.borrow().read();
    let catalog = snapshot.catalogs.get(snapshot.provider).clone();
    let page = {
        let mut ui = state.borrow_mut();
        let pages = (catalog.models.len().div_ceil(8)).max(1);
        ui.page = ui.page.min(pages - 1);
        ui.page
    };
    let pages = (catalog.models.len().div_ceil(8)).max(1);
    let mut controls: Vec<UiControl> = Vec::new();
    for (index, item) in catalog.models.iter().skip(page * 8).take(8).enumerate() {
        let state_pick = Rc::clone(state);
        let controller_close = Rc::clone(controller);
        let model_id = item.id.clone();
        controls.push(UiControl {
            id: control_id(&format!("ui:llm-models:item:{index}")),
            label: item.id.clone(),
            rect: menu_row(index as i32, &MenuRowOptions::default()),
            enabled: true,
            visible: true,
            kind: UiControlKind::Button {
                on_activate: Rc::new(move |_| {
                    {
                        let mut ui = state_pick.borrow_mut();
                        ui.model = Some(model_id.clone());
                        ui.effort = Some(None);
                        ui.status = "Save settings to use this model.".to_string();
                    }
                    controller_close.borrow_mut().close_menu();
                }),
            },
        });
    }
    if catalog.models.is_empty() {
        let label = if matches!(catalog.status, LlmCatalogStatus::Loading) {
            "Loading models..."
        } else {
            "No models loaded. Use Refresh models."
        };
        controls.push(UiControl {
            id: control_id("ui:llm-models:empty"),
            label: label.to_string(),
            rect: menu_row(0, &MenuRowOptions::default()),
            enabled: false,
            visible: true,
            kind: UiControlKind::Button {
                on_activate: Rc::new(|_| {}),
            },
        });
    }
    {
        let state_previous = Rc::clone(state);
        controls.push(UiControl {
            id: control_id("ui:llm-models:previous"),
            label: "Previous page".to_string(),
            rect: menu_row(8, &MenuRowOptions::default()),
            enabled: page > 0,
            visible: true,
            kind: UiControlKind::Button {
                on_activate: Rc::new(move |_| {
                    let mut ui = state_previous.borrow_mut();
                    if ui.page > 0 {
                        ui.page -= 1;
                    }
                }),
            },
        });
    }
    {
        let state_next = Rc::clone(state);
        controls.push(UiControl {
            id: control_id("ui:llm-models:next"),
            label: "Next page".to_string(),
            rect: menu_row(9, &MenuRowOptions::default()),
            enabled: page + 1 < pages,
            visible: true,
            kind: UiControlKind::Button {
                on_activate: Rc::new(move |_| {
                    state_next.borrow_mut().page += 1;
                }),
            },
        });
    }
    {
        let controller_back = Rc::clone(controller);
        controls.push(UiControl {
            id: control_id("ui:llm-models:back"),
            label: "Back".to_string(),
            rect: menu_row(11, &MenuRowOptions::default()),
            enabled: true,
            visible: true,
            kind: UiControlKind::Button {
                on_activate: Rc::new(move |_| {
                    controller_back.borrow_mut().close_menu();
                }),
            },
        });
    }
    UiMenu {
        scroll: None,
        id: menu_id("menu:settings:llm-models"),
        title: format!("Models ({}/{pages})", page + 1),
        full_screen: false,
        controls,
        on_open: Rc::new(|_| {}),
        on_close: Rc::new(|_| {}),
    }
}

/// Register the LLM settings root plus the models picker.
///
/// Factories re-read the service on every build; control callbacks re-enter
/// the controller through the shared handle, so callers must not hold a borrow
/// across input or draw calls that activate those controls.
pub fn register_llm_settings_menu(
    controller: &Rc<RefCell<NativeUiController>>,
    service: Rc<RefCell<dyn LlmSettingsUi>>,
) -> LlmMenus {
    let root = menu_id("menu:settings:llm");
    let models = menu_id("menu:settings:llm-models");
    let state = Rc::new(RefCell::new(LlmUiState::new()));
    {
        let controller_factory = Rc::clone(controller);
        let service_factory = Rc::clone(&service);
        let state_factory = Rc::clone(&state);
        controller.borrow_mut().register(
            root.clone(),
            Rc::new(move || root_menu(&controller_factory, &service_factory, &state_factory)),
        );
    }
    {
        let controller_factory = Rc::clone(controller);
        let service_factory = Rc::clone(&service);
        let state_factory = Rc::clone(&state);
        controller.borrow_mut().register(
            models.clone(),
            Rc::new(move || models_menu(&controller_factory, &service_factory, &state_factory)),
        );
    }
    LlmMenus {
        root,
        controller: Rc::clone(controller),
        service,
        state,
        models,
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::{IdentityOwner, SeatId};

    use super::*;
    use crate::ui::common::controller::headless_options;

    #[derive(Debug, Default)]
    struct MockLog {
        select: Vec<LlmProvider>,
        set_model: Vec<(LlmProvider, String)>,
        save_key: Vec<(LlmProvider, String)>,
        remove: Vec<LlmProvider>,
        save_other: Vec<(String, String)>,
        sign_in: usize,
        cancel: usize,
        refresh: Vec<LlmProvider>,
        effort: Vec<(LlmProvider, Option<String>)>,
    }

    struct MockService {
        snapshot: LlmSnapshot,
        log: Rc<RefCell<MockLog>>,
        select_err: Option<String>,
        set_model_err: Option<String>,
        save_key_err: Option<String>,
        remove_err: Option<String>,
        save_other_err: Option<String>,
        sign_in_err: Option<String>,
        refresh_err: Option<String>,
        effort_err: Option<String>,
    }

    impl MockService {
        fn new(snapshot: LlmSnapshot, log: Rc<RefCell<MockLog>>) -> Self {
            Self {
                snapshot,
                log,
                select_err: None,
                set_model_err: None,
                save_key_err: None,
                remove_err: None,
                save_other_err: None,
                sign_in_err: None,
                refresh_err: None,
                effort_err: None,
            }
        }

        fn provider_mut(&mut self, provider: LlmProvider) -> &mut LlmProviderState {
            match provider {
                LlmProvider::ChatGptSubscription => &mut self.snapshot.providers.subscription,
                LlmProvider::ChatGptApi => &mut self.snapshot.providers.api,
                LlmProvider::OtherApi => &mut self.snapshot.providers.other,
            }
        }
    }

    impl LlmSettingsUi for MockService {
        fn read(&self) -> LlmSnapshot {
            self.snapshot.clone()
        }

        fn select_provider(&mut self, provider: LlmProvider) -> Result<(), String> {
            self.log.borrow_mut().select.push(provider);
            if let Some(message) = &self.select_err {
                return Err(message.clone());
            }
            self.snapshot.provider = provider;
            Ok(())
        }

        fn set_model(&mut self, provider: LlmProvider, model: &str) -> Result<(), String> {
            self.log.borrow_mut().set_model.push((provider, model.to_string()));
            if let Some(message) = &self.set_model_err {
                return Err(message.clone());
            }
            self.provider_mut(provider).model = model.to_string();
            Ok(())
        }

        fn save_api_key(&mut self, provider: LlmProvider, key: &str) -> Result<(), String> {
            self.log.borrow_mut().save_key.push((provider, key.to_string()));
            if let Some(message) = &self.save_key_err {
                return Err(message.clone());
            }
            if provider == LlmProvider::ChatGptSubscription {
                return Err("Cannot save an API key for ChatGPT Subscription.".to_string());
            }
            self.provider_mut(provider).configured = true;
            Ok(())
        }

        fn remove_credential(&mut self, provider: LlmProvider) -> Result<(), String> {
            self.log.borrow_mut().remove.push(provider);
            if let Some(message) = &self.remove_err {
                return Err(message.clone());
            }
            self.provider_mut(provider).configured = false;
            Ok(())
        }

        fn save_other_service(&mut self, base_url: &str, model: &str) -> Result<(), String> {
            self.log
                .borrow_mut()
                .save_other
                .push((base_url.to_string(), model.to_string()));
            if let Some(message) = &self.save_other_err {
                return Err(message.clone());
            }
            self.snapshot.providers.other.base_url = base_url.to_string();
            self.snapshot.providers.other.model = model.to_string();
            Ok(())
        }

        fn sign_in_subscription(&mut self) -> Result<(), String> {
            self.log.borrow_mut().sign_in += 1;
            if let Some(message) = &self.sign_in_err {
                return Err(message.clone());
            }
            self.snapshot.providers.subscription.configured = true;
            self.snapshot.subscription_auth = LlmSubscriptionAuth::Idle;
            Ok(())
        }

        fn cancel_sign_in(&mut self) {
            self.log.borrow_mut().cancel += 1;
            self.snapshot.subscription_auth = LlmSubscriptionAuth::Idle;
        }

        fn refresh_models(&mut self, provider: LlmProvider) -> Result<(), String> {
            self.log.borrow_mut().refresh.push(provider);
            if let Some(message) = &self.refresh_err {
                return Err(message.clone());
            }
            Ok(())
        }

        fn set_reasoning_effort(&mut self, provider: LlmProvider, effort: Option<&str>) -> Result<(), String> {
            self.log
                .borrow_mut()
                .effort
                .push((provider, effort.map(str::to_string)));
            if let Some(message) = &self.effort_err {
                return Err(message.clone());
            }
            if provider == self.snapshot.provider {
                self.snapshot.reasoning_effort = effort.map(str::to_string);
            }
            Ok(())
        }
    }

    fn provider_state(configured: bool, model: &str, base_url: &str) -> LlmProviderState {
        LlmProviderState {
            configured,
            model: model.to_string(),
            base_url: base_url.to_string(),
        }
    }

    fn catalog(models: Vec<LlmModel>, status: LlmCatalogStatus) -> LlmCatalog {
        LlmCatalog { models, status }
    }

    fn model(id: &str, efforts: &[&str]) -> LlmModel {
        LlmModel {
            id: id.to_string(),
            reasoning_efforts: efforts.iter().map(|effort| (*effort).to_string()).collect(),
        }
    }

    fn empty_snapshot() -> LlmSnapshot {
        let idle = || catalog(Vec::new(), LlmCatalogStatus::Idle);
        LlmSnapshot {
            provider: LlmProvider::ChatGptSubscription,
            providers: LlmProviders {
                subscription: provider_state(false, "", ""),
                api: provider_state(false, "", ""),
                other: provider_state(false, "", ""),
            },
            catalogs: LlmCatalogs {
                subscription: idle(),
                api: idle(),
                other: idle(),
            },
            subscription_auth: LlmSubscriptionAuth::Idle,
            reasoning_effort: None,
            errors: Vec::new(),
        }
    }

    fn harness(snapshot: LlmSnapshot) -> (LlmMenus, Rc<RefCell<MockLog>>, SeatId) {
        harness_with(MockService::new(snapshot, Rc::new(RefCell::new(MockLog::default()))))
    }

    fn harness_with(service: MockService) -> (LlmMenus, Rc<RefCell<MockLog>>, SeatId) {
        let owner = IdentityOwner::create("llm-test").unwrap();
        let seat = owner.seat(0);
        let controller = Rc::new(RefCell::new(NativeUiController::new(headless_options(seat.clone()))));
        let log = Rc::clone(&service.log);
        let service: Rc<RefCell<dyn LlmSettingsUi>> = Rc::new(RefCell::new(service));
        let menus = register_llm_settings_menu(&controller, service);
        (menus, log, seat)
    }

    fn root(menus: &LlmMenus) -> UiMenu {
        root_menu(&menus.controller, &menus.service, &menus.state)
    }

    fn models(menus: &LlmMenus) -> UiMenu {
        models_menu(&menus.controller, &menus.service, &menus.state)
    }

    fn find<'a>(menu: &'a UiMenu, id: &str) -> &'a UiControl {
        menu.controls.iter().find(|control| control.id.as_str() == id).unwrap()
    }

    fn find_opt<'a>(menu: &'a UiMenu, id: &str) -> Option<&'a UiControl> {
        menu.controls.iter().find(|control| control.id.as_str() == id)
    }

    fn activate(control: &UiControl, seat: &SeatId) {
        let UiControlKind::Button { on_activate } = &control.kind else {
            panic!("expected button {}", control.id.as_str());
        };
        on_activate(seat.clone());
    }

    fn choose(control: &UiControl, seat: &SeatId, value: &str) {
        let UiControlKind::Choice { on_select, .. } = &control.kind else {
            panic!("expected choice {}", control.id.as_str());
        };
        on_select(seat.clone(), value);
    }

    fn type_text(control: &UiControl, seat: &SeatId, value: &str) {
        let UiControlKind::TextEntry { on_change, .. } = &control.kind else {
            panic!("expected text entry {}", control.id.as_str());
        };
        on_change(seat.clone(), value);
    }

    fn selected(control: &UiControl) -> Option<String> {
        let UiControlKind::Choice { selected, .. } = &control.kind else {
            panic!("expected choice {}", control.id.as_str());
        };
        selected.clone()
    }

    fn choices(control: &UiControl) -> Vec<(String, String)> {
        let UiControlKind::Choice { choices, .. } = &control.kind else {
            panic!("expected choice {}", control.id.as_str());
        };
        choices
            .iter()
            .map(|choice| (choice.id.clone(), choice.label.clone()))
            .collect()
    }

    #[test]
    fn provider_ids_round_trip() {
        assert_eq!(LlmProvider::all().len(), 3);
        for provider in LlmProvider::all() {
            assert_eq!(LlmProvider::parse(provider.as_str()), Some(*provider));
        }
        assert_eq!(LlmProvider::parse("other"), None);
        assert_eq!(LlmProvider::ChatGptSubscription.as_str(), "chatgpt-subscription");
        assert_eq!(LlmProvider::ChatGptApi.as_str(), "chatgpt-api");
        assert_eq!(LlmProvider::OtherApi.as_str(), "other-api");
    }

    #[test]
    fn wrap_matches_donor() {
        assert!(wrap_message("").is_empty());
        assert_eq!(wrap_message("hi"), vec!["hi".to_string()]);
        let exact = "a".repeat(54);
        assert_eq!(wrap_message(&exact), vec![exact.clone()]);
        let over = "b".repeat(55);
        assert_eq!(wrap_message(&over), vec!["b".repeat(54), "b".to_string()]);
        let words = "Loading models... Settings can still be saved.";
        assert_eq!(wrap_message(words), vec![words.to_string()]);
        // Exact donor `match` outputs (trailing spaces kept; the menu trims).
        let row = "word ".repeat(11);
        assert_eq!(
            wrap_message("word ".repeat(30).trim_end()),
            vec![row.clone(), row.clone(), "word ".repeat(8).trim_end().to_string()]
        );
        assert_eq!(
            wrap_message("word ".repeat(40).trim_end()),
            vec![
                row.clone(),
                row.clone(),
                row.clone(),
                "word ".repeat(7).trim_end().to_string()
            ]
        );
        let split_word = "a".repeat(60) + " tail";
        assert_eq!(
            wrap_message(&split_word),
            vec!["a".repeat(54), "aaaaaa tail".to_string()]
        );
    }

    #[test]
    fn root_rows_subscription_idle() {
        let (menus, _, _) = harness(empty_snapshot());
        assert_eq!(menus.root.as_str(), "menu:settings:llm");
        let menu = root(&menus);
        assert_eq!(menu.title, "LLM options");
        assert!(!menu.full_screen);
        let provider = find(&menu, "ui:llm:provider");
        assert!(provider.enabled);
        assert_eq!(selected(provider), Some("chatgpt-subscription".to_string()));
        assert_eq!(find(&menu, "ui:llm:status").label, "No credential stored");
        assert!(!find(&menu, "ui:llm:status").enabled);
        assert_eq!(find(&menu, "ui:llm:model").label, "Model: Select a model");
        let effort = find(&menu, "ui:llm:effort");
        assert!(!effort.enabled);
        assert_eq!(selected(effort), Some(String::new()));
        assert!(find(&menu, "ui:llm:signin").enabled);
        assert!(!find(&menu, "ui:llm:cancel-signin").enabled);
        assert!(!find(&menu, "ui:llm:signout").enabled);
        assert!(find_opt(&menu, "ui:llm:key").is_none());
        assert!(find_opt(&menu, "ui:llm:base-url").is_none());
        assert!(find(&menu, "ui:llm:save").enabled);
        assert!(!find(&menu, "ui:llm:refresh").enabled);
        let save = find(&menu, "ui:llm:save");
        assert_eq!((save.rect.x, save.rect.width), (64.0, 248.0));
        let refresh = find(&menu, "ui:llm:refresh");
        assert_eq!((refresh.rect.x, refresh.rect.width), (328.0, 248.0));
        assert_eq!(
            find(&menu, "ui:llm:message:0").label,
            "Select a model after signing in or saving an API key."
        );
        assert!(find_opt(&menu, "ui:llm:message:1").is_none());
        assert!(find(&menu, "ui:llm:back").enabled);
    }

    #[test]
    fn root_rows_subscription_pending() {
        let mut snapshot = empty_snapshot();
        snapshot.subscription_auth = LlmSubscriptionAuth::Pending;
        snapshot.providers.subscription.configured = true;
        let (menus, _, _) = harness(snapshot);
        let menu = root(&menus);
        assert!(!find(&menu, "ui:llm:provider").enabled);
        assert_eq!(find(&menu, "ui:llm:status").label, "Waiting for browser sign-in");
        assert!(!find(&menu, "ui:llm:signin").enabled);
        assert!(find(&menu, "ui:llm:cancel-signin").enabled);
        assert!(!find(&menu, "ui:llm:save").enabled);
        assert!(!find(&menu, "ui:llm:refresh").enabled);
    }

    #[test]
    fn root_rows_api_configured() {
        let mut snapshot = empty_snapshot();
        snapshot.provider = LlmProvider::ChatGptApi;
        snapshot.providers.api = provider_state(true, "m1", "");
        snapshot.catalogs.api = catalog(
            vec![model("m1", &["low", "high"]), model("m2", &[])],
            LlmCatalogStatus::Ready,
        );
        snapshot.reasoning_effort = Some("low".to_string());
        let (menus, _, _) = harness(snapshot);
        let menu = root(&menus);
        assert_eq!(find(&menu, "ui:llm:status").label, "Credential stored");
        assert_eq!(find(&menu, "ui:llm:model").label, "Model: m1");
        let effort = find(&menu, "ui:llm:effort");
        assert!(effort.enabled);
        assert_eq!(selected(effort), Some("low".to_string()));
        assert_eq!(
            choices(effort),
            vec![
                (String::new(), "Model default".to_string()),
                ("low".to_string(), "low".to_string()),
                ("high".to_string(), "high".to_string()),
            ]
        );
        let key = find(&menu, "ui:llm:key");
        assert!(key.enabled);
        let UiControlKind::TextEntry {
            masked,
            text,
            maximum_length,
            ..
        } = &key.kind
        else {
            panic!("expected text entry");
        };
        assert!(*masked);
        assert_eq!(*maximum_length, 4096);
        assert!(text.is_empty());
        assert!(find(&menu, "ui:llm:remove-key").enabled);
        assert!(find(&menu, "ui:llm:refresh").enabled);
        assert!(find_opt(&menu, "ui:llm:signin").is_none());
        assert!(find_opt(&menu, "ui:llm:base-url").is_none());
        assert_eq!(find(&menu, "ui:llm:message:0").label, "Console: llm_ask or llm_exec");
    }

    #[test]
    fn root_rows_other() {
        let mut snapshot = empty_snapshot();
        snapshot.provider = LlmProvider::OtherApi;
        snapshot.providers.other = provider_state(true, "om", "https://example.com");
        let (menus, _, _) = harness(snapshot);
        let menu = root(&menus);
        let base = find(&menu, "ui:llm:base-url");
        let UiControlKind::TextEntry {
            masked,
            text,
            maximum_length,
            ..
        } = &base.kind
        else {
            panic!("expected text entry");
        };
        assert!(!masked);
        assert_eq!(*maximum_length, 512);
        assert_eq!(text, "https://example.com");
        assert!(!find(&menu, "ui:llm:transport").enabled);
        assert_eq!(
            find(&menu, "ui:llm:transport").label,
            "OpenAI-compatible Chat Completions"
        );
    }

    #[test]
    fn message_precedence_matches_donor() {
        let mut snapshot = empty_snapshot();
        snapshot.provider = LlmProvider::ChatGptApi;
        snapshot.providers.api = provider_state(true, "m1", "");
        snapshot.catalogs.api = catalog(vec![model("m1", &[])], LlmCatalogStatus::Ready);
        snapshot.subscription_auth = LlmSubscriptionAuth::Error {
            message: "auth boom".to_string(),
        };
        snapshot.errors = vec![LlmError {
            message: "disk boom".to_string(),
        }];
        let (menus, _, _) = harness(snapshot);
        menus.state.borrow_mut().status = "Saved".to_string();
        assert_eq!(find(&root(&menus), "ui:llm:message:0").label, "Saved");
        menus.state.borrow_mut().status.clear();
        assert_eq!(find(&root(&menus), "ui:llm:message:0").label, "auth boom");
        menus.service.borrow_mut().cancel_sign_in();
        assert_eq!(find(&root(&menus), "ui:llm:message:0").label, "disk boom");
    }

    #[test]
    fn catalog_loading_and_error_override_status_fallbacks() {
        let mut snapshot = empty_snapshot();
        snapshot.provider = LlmProvider::ChatGptApi;
        snapshot.providers.api = provider_state(true, "m1", "");
        snapshot.catalogs.api = catalog(vec![model("m1", &[])], LlmCatalogStatus::Loading);
        let (menus, _, _) = harness(snapshot);
        menus.state.borrow_mut().status = "Saved".to_string();
        let menu = root(&menus);
        assert_eq!(
            find(&menu, "ui:llm:message:0").label,
            "Loading models... Settings can still be saved."
        );
        menus.state.borrow_mut().status.clear();
        menus.state.borrow_mut().busy = true;
        menus.state.borrow_mut().status = "Working...".to_string();
        assert_eq!(find(&root(&menus), "ui:llm:message:0").label, "Working...");
        menus.state.borrow_mut().busy = false;
        menus.state.borrow_mut().status.clear();
        assert_eq!(
            find(&root(&menus), "ui:llm:message:0").label,
            "Loading models... Settings can still be saved."
        );
        let mut err = empty_snapshot();
        err.provider = LlmProvider::ChatGptApi;
        err.providers.api = provider_state(true, "m1", "");
        err.catalogs.api = catalog(
            vec![model("m1", &[])],
            LlmCatalogStatus::Error {
                message: "catalog boom".to_string(),
            },
        );
        let (menus, _, _) = harness(err);
        assert_eq!(find(&root(&menus), "ui:llm:message:0").label, "catalog boom");
        let mut long = empty_snapshot();
        long.provider = LlmProvider::ChatGptApi;
        long.providers.api = provider_state(true, "m1", "");
        long.catalogs.api = catalog(
            vec![model("m1", &[])],
            LlmCatalogStatus::Error {
                message: "word ".repeat(40).trim_end().to_string(),
            },
        );
        let (menus, _, _) = harness(long);
        let menu = root(&menus);
        assert!(find_opt(&menu, "ui:llm:message:0").is_some());
        assert!(find_opt(&menu, "ui:llm:message:1").is_some());
        assert!(find_opt(&menu, "ui:llm:message:2").is_none());
    }

    #[test]
    fn effort_selection_tracks_metadata() {
        let mut snapshot = empty_snapshot();
        snapshot.provider = LlmProvider::ChatGptApi;
        snapshot.providers.api = provider_state(true, "m1", "");
        snapshot.catalogs.api = catalog(
            vec![model("m1", &["low"]), model("plain", &[])],
            LlmCatalogStatus::Ready,
        );
        let (menus, _, seat) = harness(snapshot);
        let menu = root(&menus);
        assert!(find(&menu, "ui:llm:effort").enabled);
        choose(find(&menu, "ui:llm:effort"), &seat, "low");
        assert_eq!(menus.state.borrow().effort, Some(Some("low".to_string())));
        choose(find(&root(&menus), "ui:llm:effort"), &seat, "");
        assert_eq!(menus.state.borrow().effort, Some(None));
        menus.state.borrow_mut().model = Some("plain".to_string());
        menus.state.borrow_mut().effort = None;
        let menu = root(&menus);
        assert!(!find(&menu, "ui:llm:effort").enabled);
        assert_eq!(choices(find(&menu, "ui:llm:effort")).len(), 1);
    }

    #[test]
    fn models_page_lists_eight_and_clamps() {
        let mut snapshot = empty_snapshot();
        snapshot.provider = LlmProvider::ChatGptApi;
        let items: Vec<LlmModel> = (0..12).map(|index| model(&format!("m{index:02}"), &[])).collect();
        snapshot.catalogs.api = catalog(items, LlmCatalogStatus::Ready);
        let (menus, _, seat) = harness(snapshot);
        let menu = models(&menus);
        assert_eq!(menu.title, "Models (1/2)");
        assert_eq!(find(&menu, "ui:llm-models:item:0").label, "m00");
        assert_eq!(find(&menu, "ui:llm-models:item:7").label, "m07");
        assert!(find_opt(&menu, "ui:llm-models:item:8").is_none());
        assert!(!find(&menu, "ui:llm-models:previous").enabled);
        assert!(find(&menu, "ui:llm-models:next").enabled);
        activate(find(&menu, "ui:llm-models:next"), &seat);
        let menu = models(&menus);
        assert_eq!(menu.title, "Models (2/2)");
        assert_eq!(find(&menu, "ui:llm-models:item:0").label, "m08");
        assert!(find(&menu, "ui:llm-models:previous").enabled);
        assert!(!find(&menu, "ui:llm-models:next").enabled);
        activate(find(&menu, "ui:llm-models:previous"), &seat);
        assert_eq!(models(&menus).title, "Models (1/2)");
        menus.state.borrow_mut().page = 9;
        let menu = models(&menus);
        assert_eq!(menu.title, "Models (2/2)");
        assert_eq!(menus.state.borrow().page, 1);
    }

    #[test]
    fn models_empty_shows_status_hint() {
        let (menus, _, _) = harness(empty_snapshot());
        let menu = models(&menus);
        assert_eq!(menu.title, "Models (1/1)");
        assert_eq!(
            find(&menu, "ui:llm-models:empty").label,
            "No models loaded. Use Refresh models."
        );
        assert!(!find(&menu, "ui:llm-models:empty").enabled);
        let mut loading = empty_snapshot();
        loading.provider = LlmProvider::ChatGptApi;
        loading.catalogs.api = catalog(Vec::new(), LlmCatalogStatus::Loading);
        let (menus, _, _) = harness(loading);
        assert_eq!(find(&models(&menus), "ui:llm-models:empty").label, "Loading models...");
    }

    #[test]
    fn model_button_opens_picker_and_pick_closes() {
        let mut snapshot = empty_snapshot();
        snapshot.provider = LlmProvider::ChatGptApi;
        snapshot.providers.api = provider_state(true, "", "");
        snapshot.catalogs.api = catalog(
            vec![model("picked", &["low"]), model("other", &[])],
            LlmCatalogStatus::Ready,
        );
        let (menus, _, seat) = harness(snapshot);
        menus.controller.borrow_mut().open_menu(&menus.root).unwrap();
        menus.state.borrow_mut().page = 1;
        activate(find(&root(&menus), "ui:llm:model"), &seat);
        assert_eq!(menus.state.borrow().page, 0);
        assert_eq!(menus.controller.borrow().active_menu(), Some(menus.models.clone()));
        activate(find(&models(&menus), "ui:llm-models:item:0"), &seat);
        assert_eq!(menus.state.borrow().model, Some("picked".to_string()));
        assert_eq!(menus.state.borrow().effort, Some(None));
        assert_eq!(menus.state.borrow().status, "Save settings to use this model.");
        assert_eq!(menus.controller.borrow().active_menu(), Some(menus.root.clone()));
        assert_eq!(find(&root(&menus), "ui:llm:model").label, "Model: picked");
    }

    #[test]
    fn save_api_flow_persists_model_key_effort() {
        let mut snapshot = empty_snapshot();
        snapshot.provider = LlmProvider::ChatGptApi;
        snapshot.providers.api = provider_state(false, "", "");
        snapshot.catalogs.api = catalog(vec![model("m1", &["low", "high"])], LlmCatalogStatus::Ready);
        let (menus, log, seat) = harness(snapshot);
        activate(find(&models(&menus), "ui:llm-models:item:0"), &seat);
        choose(find(&root(&menus), "ui:llm:effort"), &seat, "high");
        type_text(find(&root(&menus), "ui:llm:key"), &seat, "secret");
        activate(find(&root(&menus), "ui:llm:save"), &seat);
        let log = log.borrow();
        assert_eq!(log.set_model, vec![(LlmProvider::ChatGptApi, "m1".to_string())]);
        assert_eq!(log.effort, vec![(LlmProvider::ChatGptApi, Some("high".to_string()))]);
        assert_eq!(log.save_key, vec![(LlmProvider::ChatGptApi, "secret".to_string())]);
        assert_eq!(log.refresh, vec![LlmProvider::ChatGptApi]);
        let ui = menus.state.borrow();
        assert!(!ui.busy);
        assert_eq!(ui.status, "Saved");
        assert!(ui.key.is_empty());
        assert_eq!(ui.model, None);
        assert_eq!(ui.effort, None);
        let saved = menus.service.borrow().read();
        assert_eq!(saved.providers.api.model, "m1");
        assert!(saved.providers.api.configured);
        assert_eq!(saved.reasoning_effort, Some("high".to_string()));
    }

    #[test]
    fn save_other_flow_uses_service_shape() {
        let mut snapshot = empty_snapshot();
        snapshot.provider = LlmProvider::OtherApi;
        snapshot.providers.other = provider_state(true, "old", "https://old.example");
        snapshot.catalogs.other = catalog(vec![model("nm", &[])], LlmCatalogStatus::Ready);
        let (menus, log, seat) = harness(snapshot);
        activate(find(&models(&menus), "ui:llm-models:item:0"), &seat);
        type_text(find(&root(&menus), "ui:llm:base-url"), &seat, "https://new.example");
        activate(find(&root(&menus), "ui:llm:save"), &seat);
        let log = log.borrow();
        assert_eq!(
            log.save_other,
            vec![("https://new.example".to_string(), "nm".to_string())]
        );
        assert!(log.set_model.is_empty());
        assert_eq!(log.effort, vec![(LlmProvider::OtherApi, None)]);
        assert_eq!(log.refresh, vec![LlmProvider::OtherApi]);
        assert_eq!(menus.state.borrow().status, "Saved");
    }

    #[test]
    fn save_empty_model_skips_set_and_refreshes_when_idle() {
        let mut snapshot = empty_snapshot();
        snapshot.provider = LlmProvider::ChatGptApi;
        snapshot.providers.api = provider_state(true, "", "");
        snapshot.catalogs.api = catalog(Vec::new(), LlmCatalogStatus::Idle);
        let (menus, log, seat) = harness(snapshot);
        activate(find(&root(&menus), "ui:llm:save"), &seat);
        let log = log.borrow();
        assert!(log.set_model.is_empty());
        assert!(log.effort.is_empty());
        assert!(log.save_key.is_empty());
        assert_eq!(log.refresh, vec![LlmProvider::ChatGptApi]);
        assert_eq!(
            menus.state.borrow().status,
            "Saved. Select a model to use console requests."
        );
    }

    #[test]
    fn save_error_shows_message_and_keeps_drafts() {
        let mut snapshot = empty_snapshot();
        snapshot.provider = LlmProvider::ChatGptApi;
        snapshot.providers.api = provider_state(true, "", "");
        snapshot.catalogs.api = catalog(vec![model("m1", &["low"])], LlmCatalogStatus::Ready);
        let mut service = MockService::new(snapshot, Rc::new(RefCell::new(MockLog::default())));
        service.set_model_err = Some("bad model".to_string());
        let (menus, log, seat) = harness_with(service);
        menus.state.borrow_mut().model = Some("m1".to_string());
        menus.state.borrow_mut().effort = Some(Some("low".to_string()));
        type_text(find(&root(&menus), "ui:llm:key"), &seat, "k");
        activate(find(&root(&menus), "ui:llm:save"), &seat);
        let ui = menus.state.borrow();
        assert!(!ui.busy);
        assert_eq!(ui.status, "bad model");
        assert_eq!(ui.model, Some("m1".to_string()));
        assert_eq!(ui.effort, Some(Some("low".to_string())));
        assert!(ui.key.is_empty());
        let log = log.borrow();
        assert_eq!(log.set_model.len(), 1);
        assert!(log.effort.is_empty());
        assert!(log.save_key.is_empty());
        assert!(log.refresh.is_empty());
    }

    #[test]
    fn signin_success_explains_next_step() {
        for (selected, draft, expected) in [
            ("", None, "Signed in. Select a model, then save settings."),
            ("", Some("d1"), "Signed in. Save settings to use this model."),
            ("s1", None, "Signed in."),
        ] {
            let mut snapshot = empty_snapshot();
            snapshot.providers.subscription = provider_state(false, selected, "");
            let (menus, log, seat) = harness(snapshot);
            if let Some(model) = draft {
                menus.state.borrow_mut().model = Some(model.to_string());
            }
            menus.state.borrow_mut().key = "stale".to_string();
            activate(find(&root(&menus), "ui:llm:signin"), &seat);
            assert_eq!(menus.state.borrow().status, expected);
            assert!(menus.state.borrow().key.is_empty());
            assert_eq!(menus.state.borrow().model.as_deref(), draft);
            assert_eq!(log.borrow().sign_in, 1);
            assert_eq!(log.borrow().refresh, vec![LlmProvider::ChatGptSubscription]);
        }
    }

    #[test]
    fn signin_error_shows_message() {
        let mut service = MockService::new(empty_snapshot(), Rc::new(RefCell::new(MockLog::default())));
        service.sign_in_err = Some("browser closed".to_string());
        let (menus, _, seat) = harness_with(service);
        activate(find(&root(&menus), "ui:llm:signin"), &seat);
        assert_eq!(menus.state.borrow().status, "browser closed");
        assert!(!menus.state.borrow().busy);
    }

    #[test]
    fn cancel_signin_resets_and_bumps_generation() {
        let mut snapshot = empty_snapshot();
        snapshot.subscription_auth = LlmSubscriptionAuth::Pending;
        let (menus, log, seat) = harness(snapshot);
        {
            let mut ui = menus.state.borrow_mut();
            ui.key = "k".to_string();
            ui.model = Some("m".to_string());
            ui.base_url = Some("b".to_string());
            ui.effort = Some(Some("e".to_string()));
            ui.busy = true;
            ui.status = "Working...".to_string();
        }
        activate(find(&root(&menus), "ui:llm:cancel-signin"), &seat);
        let ui = menus.state.borrow();
        assert_eq!(ui.generation, 1);
        assert!(!ui.busy);
        assert!(ui.key.is_empty());
        assert_eq!(ui.model, None);
        assert_eq!(ui.base_url, None);
        assert_eq!(ui.effort, None);
        assert_eq!(ui.status, "Sign-in canceled");
        assert_eq!(log.borrow().cancel, 1);
        assert_eq!(
            menus.service.borrow().read().subscription_auth,
            LlmSubscriptionAuth::Idle
        );
    }

    #[test]
    fn open_clears_and_refreshes_close_cancels() {
        let mut snapshot = empty_snapshot();
        snapshot.provider = LlmProvider::ChatGptApi;
        snapshot.providers.api = provider_state(true, "m", "");
        let (menus, log, seat) = harness(snapshot);
        {
            let mut ui = menus.state.borrow_mut();
            ui.key = "k".to_string();
            ui.model = Some("m".to_string());
            ui.status = "stale".to_string();
        }
        (root(&menus).on_open)(seat.clone());
        assert!(menus.state.borrow().key.is_empty());
        assert_eq!(menus.state.borrow().model, None);
        assert!(menus.state.borrow().status.is_empty());
        assert_eq!(log.borrow().refresh, vec![LlmProvider::ChatGptApi]);
        {
            let mut ui = menus.state.borrow_mut();
            ui.key = "k".to_string();
            ui.busy = true;
            ui.status = "busy".to_string();
        }
        (root(&menus).on_close)(seat.clone());
        let ui = menus.state.borrow();
        assert_eq!(ui.generation, 1);
        assert!(ui.key.is_empty());
        assert!(!ui.busy);
        assert!(ui.status.is_empty());
        assert_eq!(log.borrow().cancel, 1);
    }

    #[test]
    fn provider_select_clears_and_refreshes() {
        let mut snapshot = empty_snapshot();
        snapshot.providers.api = provider_state(true, "m", "");
        let (menus, log, seat) = harness(snapshot);
        menus.state.borrow_mut().key = "k".to_string();
        menus.state.borrow_mut().model = Some("m".to_string());
        choose(find(&root(&menus), "ui:llm:provider"), &seat, "chatgpt-api");
        assert_eq!(log.borrow().select, vec![LlmProvider::ChatGptApi]);
        assert_eq!(menus.service.borrow().read().provider, LlmProvider::ChatGptApi);
        assert!(menus.state.borrow().key.is_empty());
        assert_eq!(menus.state.borrow().model, None);
        assert!(menus.state.borrow().status.is_empty());
        assert_eq!(log.borrow().refresh, vec![LlmProvider::ChatGptApi]);
        choose(find(&root(&menus), "ui:llm:provider"), &seat, "bogus");
        assert_eq!(log.borrow().select.len(), 1);
        menus.state.borrow_mut().busy = true;
        choose(find(&root(&menus), "ui:llm:provider"), &seat, "other-api");
        assert_eq!(log.borrow().select.len(), 1);
    }

    #[test]
    fn remove_credential_and_refresh_button() {
        let mut snapshot = empty_snapshot();
        snapshot.provider = LlmProvider::ChatGptApi;
        snapshot.providers.api = provider_state(true, "m", "");
        let (menus, log, seat) = harness(snapshot);
        activate(find(&root(&menus), "ui:llm:remove-key"), &seat);
        assert_eq!(log.borrow().remove, vec![LlmProvider::ChatGptApi]);
        assert_eq!(menus.state.borrow().status, "Saved");
        assert!(!menus.service.borrow().read().providers.api.configured);
        activate(find(&root(&menus), "ui:llm:refresh"), &seat);
        assert!(log.borrow().refresh.is_empty());
        menus
            .service
            .borrow_mut()
            .save_api_key(LlmProvider::ChatGptApi, "k")
            .unwrap();
        log.borrow_mut().save_key.clear();
        activate(find(&root(&menus), "ui:llm:refresh"), &seat);
        assert_eq!(log.borrow().refresh, vec![LlmProvider::ChatGptApi]);
    }

    #[test]
    fn dispose_unregisters_and_cancels() {
        let (menus, log, _) = harness(empty_snapshot());
        menus.state.borrow_mut().key = "k".to_string();
        assert!(menus.controller.borrow().is_registered(&menus.root));
        assert!(menus.controller.borrow().is_registered(&menus.models));
        let controller = Rc::clone(&menus.controller);
        let root = menus.root.clone();
        let models = menus.models.clone();
        menus.dispose();
        assert!(!controller.borrow().is_registered(&root));
        assert!(!controller.borrow().is_registered(&models));
        assert_eq!(log.borrow().cancel, 1);
    }
}
