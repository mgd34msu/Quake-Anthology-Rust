//! Settings menus and bindings.
//!
//! Donor provenance: `src/ui/settings/index.ts` in full. Live options retain
//! Q1/Q2 named cvars and Q3 archived/latched cvar behavior; categories retain
//! native controls in one scrollable view per category.

pub mod accessibility;
pub mod action_catalog;
pub mod bindings;
pub mod console;
pub mod gameplay;
pub mod gyro;
pub mod images;
pub mod input_routing;
pub mod language;
pub mod llm;
pub mod local_lobby;
pub mod ranking_account;
pub mod rankings;
pub mod rotation;
pub mod server;
pub mod services;

// Re-export the donor `export *` modules; several are still sibling-owned
// skeletons, so empty glob imports are expected until those ports land.
#[allow(unused_imports)]
pub use bindings::*;
#[allow(unused_imports)]
pub use gyro::*;
#[allow(unused_imports)]
pub use input_routing::*;
#[allow(unused_imports)]
pub use services::*;

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use qa_core::identity::SeatId;
use qa_core::math::Vec2;

use self::accessibility::{read_ui_preferences, write_ui_preferences};
use self::llm::{register_llm_settings_menu, LlmMenus, LlmSettingsUi};
use crate::error::ClientError;
use crate::text::draw2d::Rect;
use crate::ui::common::controller::NativeUiController;
use crate::ui::common::layout::{menu_row, MenuRowOptions};
use crate::ui::types::{
    CommandDialect, InterfaceColorMode, SeatUiController as _, Typeface, UiChoice, UiControl, UiControlId,
    UiControlKind, UiMenu, UiMenuId, UiMenuScroll, UiPreferenceValues, DEFAULT_UI_PREFERENCES,
};

/// Q3 cvar flag word for read-only cvars (`CvarFlag.ReadOnly`).
pub const CVAR_FLAG_READONLY: u32 = 64;
/// Q3 cvar flag word for init cvars (`CvarFlag.Init`).
pub const CVAR_FLAG_INIT: u32 = 16;
/// Q2 cvar flag word for no-set cvars (`Q2CvarFlag.NoSet`).
pub const Q2_CVAR_FLAG_NOSET: u32 = 8;

/// Settings category selecting which category menu owns a binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingCategory {
    /// Display options.
    Display,
    /// Graphics options.
    Video,
    /// Audio options.
    Audio,
    /// Controls options.
    Input,
    /// Network options.
    Network,
    /// Accessibility options.
    Accessibility,
    /// Language options.
    Language,
}

impl SettingCategory {
    /// Donor category id.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            SettingCategory::Display => "display",
            SettingCategory::Video => "video",
            SettingCategory::Audio => "audio",
            SettingCategory::Input => "input",
            SettingCategory::Network => "network",
            SettingCategory::Accessibility => "accessibility",
            SettingCategory::Language => "language",
        }
    }

    /// Category menu title.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            SettingCategory::Display => "Display",
            SettingCategory::Video => "Graphics",
            SettingCategory::Audio => "Audio",
            SettingCategory::Input => "Controls",
            SettingCategory::Network => "Network",
            SettingCategory::Accessibility => "Accessibility",
            SettingCategory::Language => "Language",
        }
    }

    /// Every category in root-menu order.
    #[must_use]
    pub fn all() -> &'static [SettingCategory] {
        &[
            SettingCategory::Display,
            SettingCategory::Video,
            SettingCategory::Audio,
            SettingCategory::Input,
            SettingCategory::Network,
            SettingCategory::Accessibility,
            SettingCategory::Language,
        ]
    }
}

/// Draft commit hooks for a text-entry binding.
#[derive(Clone)]
pub struct SettingCommit {
    /// Commit the submitted value.
    pub submit: Rc<dyn Fn(&str)>,
    /// Discard the draft.
    pub cancel: Rc<dyn Fn()>,
}

/// Live read/write surface behind one [`SettingBinding`].
#[derive(Clone)]
pub enum SettingBindingKind {
    /// Boolean toggle.
    Toggle {
        /// Read the current value.
        read: Rc<dyn Fn() -> bool>,
        /// Write a new value.
        write: Rc<dyn Fn(bool)>,
    },
    /// Numeric slider.
    Slider {
        /// Read the current value.
        read: Rc<dyn Fn() -> f32>,
        /// Write a new value.
        write: Rc<dyn Fn(f32)>,
        /// Minimum value.
        minimum: f32,
        /// Maximum value.
        maximum: f32,
        /// Step value.
        step: f32,
        /// Optional value-label formatter.
        format_value: Option<Rc<dyn Fn(f32) -> String>>,
    },
    /// Choice list.
    Choice {
        /// Read the selected choice id.
        read: Rc<dyn Fn() -> String>,
        /// Write the selected choice id.
        write: Rc<dyn Fn(&str)>,
        /// List the available choices.
        choices: Rc<dyn Fn() -> Vec<UiChoice>>,
    },
    /// Text entry.
    TextEntry {
        /// Read the current text.
        read: Rc<dyn Fn() -> String>,
        /// Write the current text (or draft when `commit` is set).
        write: Rc<dyn Fn(&str)>,
        /// Maximum length in characters.
        maximum_length: usize,
        /// Draft commit hooks for submit-only entries.
        commit: Option<SettingCommit>,
    },
    /// Action button.
    Button {
        /// Run the action.
        activate: Rc<dyn Fn()>,
    },
}

/// One live settings row: identity plus its read/write surface.
#[derive(Clone)]
pub struct SettingBinding {
    /// Control id.
    pub id: UiControlId,
    /// Row label.
    pub label: String,
    /// Owning category menu.
    pub category: SettingCategory,
    /// Whether the row is currently enabled.
    pub enabled: Rc<dyn Fn() -> bool>,
    /// Read/write surface.
    pub kind: SettingBindingKind,
}

/// Build a seat-owned [`UiControl`] for one binding.
///
/// Every callback rejects seats other than `seat` by panicking, mirroring the
/// donor throw; a control handed to the wrong seat is a caller bug.
#[must_use]
pub fn setting_control(binding: &SettingBinding, rect: &Rect, seat: SeatId) -> UiControl {
    let check = |actual: &SeatId, seat: &SeatId| {
        if actual != seat {
            panic!("Settings control belongs to another seat");
        }
    };
    let base = |binding: &SettingBinding| (binding.id.clone(), binding.label.clone(), *rect, (binding.enabled)());
    let (id, label, rect, enabled) = base(binding);
    let kind = match &binding.kind {
        SettingBindingKind::Toggle { read, write } => {
            let seat_check = seat.clone();
            let write = Rc::clone(write);
            let checked = read();
            UiControlKind::Toggle {
                checked,
                on_change: Rc::new(move |actual, value| {
                    check(&actual, &seat_check);
                    write(value);
                }),
            }
        }
        SettingBindingKind::Slider {
            read,
            write,
            minimum,
            maximum,
            step,
            format_value,
        } => {
            let seat_check = seat.clone();
            let write = Rc::clone(write);
            let value = read();
            let value_label = format_value.as_ref().map(|format| format(value));
            UiControlKind::Slider {
                minimum: *minimum,
                maximum: *maximum,
                step: *step,
                value,
                value_label,
                on_change: Rc::new(move |actual, value| {
                    check(&actual, &seat_check);
                    write(value);
                }),
            }
        }
        SettingBindingKind::Choice { read, write, choices } => {
            let seat_check = seat.clone();
            let write = Rc::clone(write);
            UiControlKind::Choice {
                choices: choices(),
                selected: Some(read()),
                on_select: Rc::new(move |actual, value| {
                    check(&actual, &seat_check);
                    write(value);
                }),
            }
        }
        SettingBindingKind::TextEntry {
            read,
            write,
            maximum_length,
            commit,
        } => {
            let seat_check = seat.clone();
            let change_write = Rc::clone(write);
            let submit_write = Rc::clone(write);
            let commit = commit.clone();
            UiControlKind::TextEntry {
                masked: false,
                text: read(),
                maximum_length: *maximum_length,
                on_change: Rc::new(move |actual, value| {
                    check(&actual, &seat_check);
                    change_write(value);
                }),
                on_submit: Rc::new(move |actual, value| {
                    check(&actual, &seat);
                    match &commit {
                        None => submit_write(value),
                        Some(commit) => (commit.submit)(value),
                    }
                }),
            }
        }
        SettingBindingKind::Button { activate } => {
            let activate = Rc::clone(activate);
            UiControlKind::Button {
                on_activate: Rc::new(move |actual| {
                    check(&actual, &seat);
                    activate();
                }),
            }
        }
    };
    UiControl {
        id,
        label,
        rect,
        enabled,
        visible: true,
        kind,
    }
}

/// Snapshot of one cvar visible to settings bindings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CvarView {
    /// Current value.
    pub value: String,
    /// Latched value awaiting a restart, if any.
    pub latched_value: Option<String>,
    /// Reset value.
    pub reset_value: String,
    /// Flag word.
    pub flags: u32,
}

/// Cvar surface settings bindings may use: lookup, write, and numeric read.
///
/// The subsystem registers its cvar and supplies the spec; menus never create
/// unused cvars.
pub trait SettingCvars {
    /// Registry dialect.
    fn dialect(&self) -> CommandDialect;
    /// Find one cvar by name.
    fn find(&self, name: &str) -> Option<CvarView>;
    /// Set one cvar.
    fn set(&self, name: &str, value: &str);
    /// Read one cvar as a number.
    fn variable_value(&self, name: &str) -> f32;
}

/// Cvar validator returning an error message for rejected values.
pub type CvarValidator = Rc<dyn Fn(&str) -> Option<String>>;

/// Registration surface for settings-owned cvars.
pub trait RegisterCvars: SettingCvars {
    /// Register one cvar with its default value.
    fn register(&mut self, name: &str, value: &str, archive: bool);
    /// Bind a validator returning an error message for rejected values.
    fn bind_validator(&mut self, name: &str, validate: CvarValidator);
}

/// Restart scope a cvar change may require.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RestartKind {
    /// Video subsystem restart.
    Video,
    /// Input subsystem restart.
    Input,
    /// Audio subsystem restart.
    Audio,
}

impl RestartKind {
    /// Donor restart name.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            RestartKind::Video => "video",
            RestartKind::Input => "input",
            RestartKind::Audio => "audio",
        }
    }

    /// Parse a donor restart name.
    pub fn parse(text: &str) -> Result<Self, ClientError> {
        match text {
            "video" => Ok(RestartKind::Video),
            "input" => Ok(RestartKind::Input),
            "audio" => Ok(RestartKind::Audio),
            _ => Err(ClientError::BadUi(format!("unknown restart kind: {text}"))),
        }
    }
}

/// Sink collecting subsystem restart requests from settings writes.
pub trait RestartSink {
    /// Request one restart.
    fn request(&mut self, kind: RestartKind);
}

/// Control shape for one cvar-backed setting.
#[derive(Debug, Clone, PartialEq)]
pub enum CvarSettingKind {
    /// Boolean toggle (`"0"`/`"1"`).
    Toggle,
    /// Numeric slider.
    Slider {
        /// Minimum value.
        minimum: f32,
        /// Maximum value.
        maximum: f32,
        /// Step value.
        step: f32,
    },
    /// Fixed choice list.
    Choice {
        /// Available choices.
        choices: Vec<UiChoice>,
    },
    /// Text entry.
    TextEntry {
        /// Maximum length in characters.
        maximum_length: usize,
        /// When true, edits stay a draft until submit.
        submit_only: bool,
    },
}

/// Spec binding one owned cvar to one settings row.
#[derive(Debug, Clone, PartialEq)]
pub struct CvarSettingSpec {
    /// Cvar name; must already be registered by its owner.
    pub name: String,
    /// Row label.
    pub label: String,
    /// Owning category menu.
    pub category: SettingCategory,
    /// Restart requested after each write, if any.
    pub restart: Option<RestartKind>,
    /// Control shape.
    pub kind: CvarSettingKind,
}

/// Parse a cvar string the way the donor `Number()` read does: blank reads as
/// zero, unparseable reads as NaN (which compares unequal to zero).
fn cvar_number(text: &str) -> f32 {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return 0.0;
    }
    trimmed.parse::<f32>().unwrap_or(f32::NAN)
}

/// Bind one owned cvar to a settings row.
///
/// Reads prefer the latched value; rows over read-only cvars report disabled.
/// Unknown choice writes panic, mirroring the donor `RangeError`.
pub fn bind_cvar_setting(
    registry: &Rc<dyn SettingCvars>,
    spec: CvarSettingSpec,
    restarts: Option<&Rc<RefCell<dyn RestartSink>>>,
) -> Result<SettingBinding, ClientError> {
    if registry.find(&spec.name).is_none() {
        return Err(ClientError::BadUi(format!("Settings cvar has no owner: {}", spec.name)));
    }
    if let Some(restart) = spec.restart {
        if restarts.is_none() {
            return Err(ClientError::BadUi(format!(
                "Settings cvar needs a {} restart owner: {}",
                restart.as_str(),
                spec.name
            )));
        }
    }
    let read_registry = Rc::clone(registry);
    let read_name = spec.name.clone();
    let read = Rc::new(move || {
        let view = read_registry.find(&read_name).unwrap_or_else(|| {
            panic!("Settings cvar was unregistered: {read_name}");
        });
        view.latched_value.unwrap_or(view.value)
    });
    let write_registry = Rc::clone(registry);
    let write_name = spec.name.clone();
    let write_restarts = restarts.cloned();
    let write_restart = spec.restart;
    let write = Rc::new(move |value: &str| {
        write_registry.set(&write_name, value);
        if let (Some(restart), Some(sink)) = (write_restart, write_restarts.as_ref()) {
            sink.borrow_mut().request(restart);
        }
    });
    let enabled_registry = Rc::clone(registry);
    let enabled_name = spec.name.clone();
    let enabled: Rc<dyn Fn() -> bool> = Rc::new(move || {
        let Some(view) = enabled_registry.find(&enabled_name) else {
            return false;
        };
        let dialect = enabled_registry.dialect();
        let readonly = if dialect == CommandDialect::Q3 {
            CVAR_FLAG_READONLY | CVAR_FLAG_INIT
        } else if dialect.is_q2() {
            Q2_CVAR_FLAG_NOSET
        } else {
            0
        };
        view.flags & readonly == 0
    });
    let id = control_id(&format!("ui:settings:{}", spec.name));
    let base = SettingBinding {
        id,
        label: spec.label.clone(),
        category: spec.category,
        enabled,
        kind: SettingBindingKind::Button {
            activate: Rc::new(|| {}),
        },
    };
    let kind = match &spec.kind {
        CvarSettingKind::Toggle => {
            let read_toggle = Rc::clone(&read);
            let write_toggle = Rc::clone(&write);
            SettingBindingKind::Toggle {
                read: Rc::new(move || cvar_number(&read_toggle()) != 0.0),
                write: Rc::new(move |value| write_toggle(if value { "1" } else { "0" })),
            }
        }
        CvarSettingKind::Slider { minimum, maximum, step } => {
            let read_slider = Rc::clone(&read);
            let write_slider = Rc::clone(&write);
            SettingBindingKind::Slider {
                read: Rc::new(move || cvar_number(&read_slider())),
                write: Rc::new(move |value| write_slider(&value.to_string())),
                minimum: *minimum,
                maximum: *maximum,
                step: *step,
                format_value: None,
            }
        }
        CvarSettingKind::Choice { choices } => {
            let read_choice = Rc::clone(&read);
            let write_choice = Rc::clone(&write);
            let listed = choices.clone();
            let valid = choices.clone();
            SettingBindingKind::Choice {
                read: Rc::new(move || read_choice()),
                write: Rc::new(move |value| {
                    if !valid.iter().any(|choice| choice.id == value) {
                        panic!("Unknown cvar setting choice");
                    }
                    write_choice(value);
                }),
                choices: Rc::new(move || listed.clone()),
            }
        }
        CvarSettingKind::TextEntry {
            maximum_length,
            submit_only,
        } => {
            if !submit_only {
                let read_text = Rc::clone(&read);
                let write_text = Rc::clone(&write);
                SettingBindingKind::TextEntry {
                    read: Rc::new(move || read_text()),
                    write: Rc::new(move |value| write_text(value)),
                    maximum_length: *maximum_length,
                    commit: None,
                }
            } else {
                let draft: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
                let read_draft = Rc::clone(&draft);
                let read_live = Rc::clone(&read);
                let write_draft = Rc::clone(&draft);
                let submit_live = Rc::clone(&write);
                let submit_draft = Rc::clone(&draft);
                let cancel_draft = Rc::clone(&draft);
                SettingBindingKind::TextEntry {
                    read: Rc::new(move || read_draft.borrow().clone().unwrap_or_else(|| read_live())),
                    write: Rc::new(move |value| {
                        *write_draft.borrow_mut() = Some(value.to_string());
                    }),
                    maximum_length: *maximum_length,
                    commit: Some(SettingCommit {
                        submit: Rc::new(move |value| {
                            submit_live(value);
                            *submit_draft.borrow_mut() = None;
                        }),
                        cancel: Rc::new(move || {
                            *cancel_draft.borrow_mut() = None;
                        }),
                    }),
                }
            }
        }
    };
    Ok(SettingBinding { kind, ..base })
}

/// Build a control id from a static template; the templates below always
/// carry the `ui:` namespace and a name part, so a failure is a programming
/// bug.
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

/// Reset-to-defaults row: label plus the apply action.
#[derive(Clone)]
pub struct SettingReset {
    /// Row and confirm-menu label.
    pub label: String,
    /// Restore defaults.
    pub apply: Rc<dyn Fn()>,
}

/// Open settings menus: the root plus one scrollable menu per used category.
pub struct SettingsMenus {
    /// Root menu id (`menu:settings:root`).
    pub root: UiMenuId,
    controller: Rc<RefCell<NativeUiController>>,
    ids: Vec<UiMenuId>,
    llm_menus: Option<LlmMenus>,
}

impl SettingsMenus {
    /// Track registered menu ids for later disposal.
    pub fn new(controller: &Rc<RefCell<NativeUiController>>, root: UiMenuId, ids: Vec<UiMenuId>) -> Self {
        Self {
            root,
            controller: Rc::clone(controller),
            ids,
            llm_menus: None,
        }
    }

    /// Unregister every tracked menu, newest first, then release the LLM menu.
    pub fn dispose(mut self) {
        for id in self.ids.iter().rev() {
            self.controller.borrow_mut().unregister(id);
        }
        if let Some(menus) = self.llm_menus.take() {
            menus.dispose();
        }
    }
}

impl std::fmt::Debug for SettingsMenus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsMenus")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

/// Register the settings root plus one scrollable category menu per category
/// that owns at least one binding.
///
/// Menu factories capture the seat by value; control callbacks re-enter the
/// controller through the shared handle, so callers must not hold a borrow
/// across input or draw calls that activate those controls.
pub fn register_settings_menus(
    controller: &Rc<RefCell<NativeUiController>>,
    bindings: &[SettingBinding],
    llm: Option<Rc<RefCell<dyn LlmSettingsUi>>>,
    reset: Option<SettingReset>,
) -> SettingsMenus {
    let root = menu_id("menu:settings:root");
    let seat = controller.borrow().seat();
    let mut ids: Vec<UiMenuId> = Vec::new();
    let llm_menus: Option<LlmMenus> = llm.map(|service| register_llm_settings_menu(controller, service));
    if let Some(reset) = reset.clone() {
        let id = menu_id("menu:settings:reset-confirm");
        let keep_controller = Rc::clone(controller);
        let apply_controller = Rc::clone(controller);
        let apply = reset.clone();
        controller.borrow_mut().register(
            id.clone(),
            Rc::new(move || {
                let keep_controller = Rc::clone(&keep_controller);
                let apply_controller = Rc::clone(&apply_controller);
                let apply = apply.clone();
                UiMenu {
                    scroll: None,
                    id: menu_id("menu:settings:reset-confirm"),
                    title: reset.label.clone(),
                    full_screen: true,
                    controls: vec![
                        UiControl {
                            id: control_id("ui:settings:keep"),
                            label: "Keep current settings".to_string(),
                            rect: menu_row(3, &MenuRowOptions::default()),
                            enabled: true,
                            visible: true,
                            kind: UiControlKind::Button {
                                on_activate: Rc::new(move |_| {
                                    keep_controller.borrow_mut().close_menu();
                                }),
                            },
                        },
                        UiControl {
                            id: control_id("ui:settings:reset-apply"),
                            label: "Restore defaults".to_string(),
                            rect: menu_row(5, &MenuRowOptions::default()),
                            enabled: true,
                            visible: true,
                            kind: UiControlKind::Button {
                                on_activate: Rc::new(move |_| {
                                    (apply.apply)();
                                    apply_controller.borrow_mut().close_menu();
                                }),
                            },
                        },
                    ],
                    on_open: Rc::new(|_| {}),
                    on_close: Rc::new(|_| {}),
                }
            }),
        );
        ids.push(id);
    }

    let back = |controller: &Rc<RefCell<NativeUiController>>| {
        let back_controller = Rc::clone(controller);
        UiControl {
            id: control_id("ui:settings:back"),
            label: "Back".to_string(),
            rect: menu_row(11, &MenuRowOptions::default()),
            enabled: true,
            visible: true,
            kind: UiControlKind::Button {
                on_activate: Rc::new(move |_| {
                    back_controller.borrow_mut().close_menu();
                }),
            },
        }
    };
    for category in SettingCategory::all() {
        let selected: Vec<SettingBinding> = bindings
            .iter()
            .filter(|binding| &binding.category == category)
            .cloned()
            .collect();
        if selected.is_empty() {
            continue;
        }
        let id = menu_id(&format!("menu:settings:{}:0", category.as_str()));
        let factory_bindings = selected.clone();
        let close_bindings = selected.clone();
        let title = category.label().to_string();
        let menu_id_factory = id.clone();
        let seat_factory = seat.clone();
        let back_controller = Rc::clone(controller);
        controller.borrow_mut().register(
            id.clone(),
            Rc::new(move || {
                let wide = MenuRowOptions {
                    width: Some(496.0),
                    ..MenuRowOptions::default()
                };
                let controls: Vec<UiControl> = factory_bindings
                    .iter()
                    .enumerate()
                    .map(|(index, binding)| {
                        setting_control(binding, &menu_row(index as i32, &wide), seat_factory.clone())
                    })
                    .collect();
                let scrolled: Vec<UiControlId> = controls.iter().map(|control| control.id.clone()).collect();
                let mut all = controls;
                let back_controller = Rc::clone(&back_controller);
                all.push(UiControl {
                    id: control_id("ui:settings:back"),
                    label: "Back".to_string(),
                    rect: menu_row(11, &MenuRowOptions::default()),
                    enabled: true,
                    visible: true,
                    kind: UiControlKind::Button {
                        on_activate: Rc::new(move |_| {
                            back_controller.borrow_mut().close_menu();
                        }),
                    },
                });
                let close_bindings = close_bindings.clone();
                UiMenu {
                    scroll: Some(UiMenuScroll {
                        rect: Rect {
                            x: 64.0,
                            y: 92.0,
                            width: 512.0,
                            height: 300.0,
                        },
                        content_height: factory_bindings.len() as f32 * 28.0,
                        controls: scrolled,
                    }),
                    id: menu_id_factory.clone(),
                    title: title.clone(),
                    full_screen: false,
                    controls: all,
                    on_open: Rc::new(|_| {}),
                    on_close: Rc::new(move |_| {
                        for binding in &close_bindings {
                            if let SettingBindingKind::TextEntry {
                                commit: Some(commit), ..
                            } = &binding.kind
                            {
                                (commit.cancel)();
                            }
                        }
                    }),
                }
            }),
        );
        ids.push(id);
    }

    let used: Vec<SettingCategory> = SettingCategory::all()
        .iter()
        .copied()
        .filter(|category| bindings.iter().any(|binding| &binding.category == category))
        .collect();
    let llm_root = llm_menus.as_ref().map(|menus| menus.root.clone());
    let reset_row = reset.clone();
    let root_factory = root.clone();
    let back_control = back(controller);
    let open_controller = Rc::clone(controller);
    controller.borrow_mut().register(
        root.clone(),
        Rc::new(move || {
            let mut controls: Vec<UiControl> = Vec::new();
            for (index, category) in used.iter().enumerate() {
                let target = menu_id(&format!("menu:settings:{}:0", category.as_str()));
                let open_controller = Rc::clone(&open_controller);
                controls.push(UiControl {
                    id: control_id(&format!("ui:settings:category:{}", category.as_str())),
                    label: category.label().to_string(),
                    rect: menu_row(index as i32, &MenuRowOptions::default()),
                    enabled: true,
                    visible: true,
                    kind: UiControlKind::Button {
                        on_activate: Rc::new(move |_| {
                            let _ = open_controller.borrow_mut().open_menu(&target);
                        }),
                    },
                });
            }
            if let Some(llm_root) = llm_root.clone() {
                let open_controller = Rc::clone(&open_controller);
                controls.push(UiControl {
                    id: control_id("ui:settings:llm"),
                    label: "LLM options".to_string(),
                    rect: menu_row(used.len() as i32, &MenuRowOptions::default()),
                    enabled: true,
                    visible: true,
                    kind: UiControlKind::Button {
                        on_activate: Rc::new(move |_| {
                            let _ = open_controller.borrow_mut().open_menu(&llm_root);
                        }),
                    },
                });
            }
            if let Some(reset) = reset_row.clone() {
                let rows = used.len() + usize::from(llm_root.is_some());
                let open_controller = Rc::clone(&open_controller);
                controls.push(UiControl {
                    id: control_id("ui:settings:reset"),
                    label: reset.label.clone(),
                    rect: menu_row(rows as i32, &MenuRowOptions::default()),
                    enabled: true,
                    visible: true,
                    kind: UiControlKind::Button {
                        on_activate: Rc::new(move |_| {
                            let target = menu_id("menu:settings:reset-confirm");
                            let _ = open_controller.borrow_mut().open_menu(&target);
                        }),
                    },
                });
            }
            controls.push(back_control.clone());
            UiMenu {
                scroll: None,
                id: root_factory.clone(),
                title: "Options".to_string(),
                full_screen: false,
                controls,
                on_open: Rc::new(|_| {}),
                on_close: Rc::new(|_| {}),
            }
        }),
    );
    ids.push(root.clone());
    let mut menus = SettingsMenus::new(controller, root, ids);
    menus.llm_menus = llm_menus;
    menus
}

/// Value service backing a group of settings rows: read the whole value,
/// apply a mutation to it.
pub trait SettingsValueService<T> {
    /// Read the current value.
    fn read(&self) -> T;
    /// Apply a mutation to the stored value.
    fn write(&self, update: &dyn Fn(&mut T));
}

/// Primary mouse sensitivity plus run behavior.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PrimaryInputSettings {
    /// Mouse sensitivity.
    pub sensitivity: f32,
    /// Pitch scale.
    pub pitch: f32,
    /// Yaw scale.
    pub yaw: f32,
    /// Invert the mouse pitch axis.
    pub invert_mouse: bool,
    /// Always run.
    pub always_run: bool,
}

/// Mouse motion behavior shared with the mouse tuning object.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MouseMotionSettings {
    /// Mouse acceleration.
    pub acceleration: f32,
    /// Mouse smoothing filter.
    pub filter: bool,
    /// Free look.
    pub free_look: bool,
    /// Look spring (Q1 only).
    pub look_spring: bool,
    /// Look strafe (Q1 only).
    pub look_strafe: bool,
}

/// Controller vibration behavior.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ControllerVibrationSettings {
    /// Controller vibration enabled.
    pub controller_vibration: bool,
    /// Vibration strength in `[0, 1]`.
    pub controller_vibration_strength: f32,
}

/// Shared vibration service behind input settings.
pub type VibrationService = Rc<dyn SettingsValueService<ControllerVibrationSettings>>;

/// Effects and music volumes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioSettings {
    /// Effects volume in `[0, 1]`.
    pub effects_volume: f32,
    /// Music volume in `[0, 1]`.
    pub music_volume: f32,
}

/// Full mouse tuning object sampled by user-command builders.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MouseTuningView {
    /// Mouse sensitivity.
    pub sensitivity: f32,
    /// Mouse acceleration.
    pub acceleration: f32,
    /// Mouse smoothing filter.
    pub filter: bool,
    /// Yaw scale.
    pub yaw: f32,
    /// Pitch scale.
    pub pitch: f32,
    /// Side-move scale.
    pub side: f32,
    /// Forward-move scale.
    pub forward: f32,
    /// Free look.
    pub free_look: bool,
    /// Look spring (Q1 only).
    pub look_spring: bool,
    /// Look strafe (Q1 only).
    pub look_strafe: bool,
    /// Invert the mouse pitch axis.
    pub invert_pitch: bool,
}

/// Default mouse tuning, ported from the donor mouse input module.
pub const DEFAULT_MOUSE_TUNING: MouseTuningView = MouseTuningView {
    sensitivity: 3.0,
    acceleration: 0.0,
    filter: false,
    yaw: 0.022,
    pitch: 0.022,
    side: 0.8,
    forward: 1.0,
    free_look: true,
    look_spring: false,
    look_strafe: false,
    invert_pitch: false,
};

fn input_numeric(
    id: &str,
    label: &str,
    minimum: f32,
    maximum: f32,
    step: f32,
    read: Rc<dyn Fn() -> f32>,
    write: Rc<dyn Fn(f32)>,
) -> SettingBinding {
    SettingBinding {
        id: control_id(&format!("ui:input:{id}")),
        label: label.to_string(),
        category: SettingCategory::Input,
        enabled: Rc::new(|| true),
        kind: SettingBindingKind::Slider {
            read,
            write,
            minimum,
            maximum,
            step,
            format_value: None,
        },
    }
}

fn input_toggle(id: &str, label: &str, read: Rc<dyn Fn() -> bool>, write: Rc<dyn Fn(bool)>) -> SettingBinding {
    SettingBinding {
        id: control_id(&format!("ui:input:{id}")),
        label: label.to_string(),
        category: SettingCategory::Input,
        enabled: Rc::new(|| true),
        kind: SettingBindingKind::Toggle { read, write },
    }
}

/// Format a sensitivity percentage the way the donor template does:
/// two decimals with trailing zeros stripped.
fn format_percent(value: f32) -> String {
    let rounded = format!("{value:.2}");
    let trimmed = rounded.trim_end_matches('0').trim_end_matches('.');
    if trimmed == "-0" || trimmed.is_empty() {
        "0%".to_string()
    } else {
        format!("{trimmed}%")
    }
}

fn mouse_axis(service: &Rc<dyn SettingsValueService<PrimaryInputSettings>>, axis: char, name: &str) -> SettingBinding {
    let pitch = axis == 'p';
    let default = if pitch {
        DEFAULT_MOUSE_TUNING.pitch
    } else {
        DEFAULT_MOUSE_TUNING.yaw
    };
    let read_service = Rc::clone(service);
    let write_service = Rc::clone(service);
    let read: Rc<dyn Fn() -> f32> = Rc::new(move || {
        let current = if pitch {
            read_service.read().pitch
        } else {
            read_service.read().yaw
        };
        current.abs() / default * 100.0
    });
    let write: Rc<dyn Fn(f32)> = Rc::new(move |value| {
        let current = if pitch {
            write_service.read().pitch
        } else {
            write_service.read().yaw
        };
        let direction = if current < 0.0 || (current == 0.0 && current.is_sign_negative()) {
            -1.0
        } else {
            1.0
        };
        let scaled = direction * default * value.clamp(0.0, 200.0) / 100.0;
        if pitch {
            write_service.write(&|settings| settings.pitch = scaled);
        } else {
            write_service.write(&|settings| settings.yaw = scaled);
        }
    });
    let id = if pitch { "mouse-pitch" } else { "mouse-yaw" };
    let mut binding = input_numeric(id, name, 0.0, 200.0, 1.0, read, write);
    if let SettingBindingKind::Slider { format_value, .. } = &mut binding.kind {
        *format_value = Some(Rc::new(format_percent));
    }
    binding
}

/// Bind mouse sensitivity, axis scales, invert, and always-run rows.
pub fn bind_primary_input_settings(service: Rc<dyn SettingsValueService<PrimaryInputSettings>>) -> Vec<SettingBinding> {
    let sensitivity_read = Rc::clone(&service);
    let sensitivity_write = Rc::clone(&service);
    let invert_read = Rc::clone(&service);
    let invert_write = Rc::clone(&service);
    let run_read = Rc::clone(&service);
    let run_write = Rc::clone(&service);
    vec![
        input_numeric(
            "sensitivity",
            "Mouse sensitivity",
            0.1,
            20.0,
            0.1,
            Rc::new(move || sensitivity_read.read().sensitivity),
            Rc::new(move |value| sensitivity_write.write(&|settings| settings.sensitivity = value)),
        ),
        mouse_axis(&service, 'y', "Horizontal sensitivity"),
        mouse_axis(&service, 'p', "Vertical sensitivity"),
        input_toggle(
            "invert-mouse",
            "Invert mouse",
            Rc::new(move || invert_read.read().invert_mouse),
            Rc::new(move |value| invert_write.write(&|settings| settings.invert_mouse = value)),
        ),
        input_toggle(
            "always-run",
            "Always run",
            Rc::new(move || run_read.read().always_run),
            Rc::new(move |value| run_write.write(&|settings| settings.always_run = value)),
        ),
    ]
}

/// Bind mouse acceleration, smoothing, and Q1 look-behavior rows.
pub fn bind_mouse_motion_settings(
    service: Rc<dyn SettingsValueService<MouseMotionSettings>>,
    spring_available: Rc<dyn Fn() -> bool>,
) -> Vec<SettingBinding> {
    let acceleration_read = Rc::clone(&service);
    let acceleration_write = Rc::clone(&service);
    let filter_read = Rc::clone(&service);
    let filter_write = Rc::clone(&service);
    let spring_read = Rc::clone(&service);
    let spring_write = Rc::clone(&service);
    let spring_enabled = Rc::clone(&service);
    let spring_gate = Rc::clone(&spring_available);
    let strafe_read = Rc::clone(&service);
    let strafe_write = Rc::clone(&service);
    let free_read = Rc::clone(&service);
    let free_write = Rc::clone(&service);
    let mut look_spring = input_toggle(
        "lookspring",
        "Look spring",
        Rc::new(move || spring_read.read().look_spring),
        Rc::new(move |value| spring_write.write(&|settings| settings.look_spring = value)),
    );
    look_spring.enabled = Rc::new(move || spring_gate() && !spring_enabled.read().free_look);
    vec![
        input_numeric(
            "acceleration",
            "Mouse acceleration",
            0.0,
            2.0,
            0.05,
            Rc::new(move || acceleration_read.read().acceleration),
            Rc::new(move |value| {
                acceleration_write.write(&|settings| settings.acceleration = value);
            }),
        ),
        input_toggle(
            "filter",
            "Mouse smoothing",
            Rc::new(move || filter_read.read().filter),
            Rc::new(move |value| filter_write.write(&|settings| settings.filter = value)),
        ),
        look_spring,
        input_toggle(
            "lookstrafe",
            "Look strafe",
            Rc::new(move || strafe_read.read().look_strafe),
            Rc::new(move |value| strafe_write.write(&|settings| settings.look_strafe = value)),
        ),
        input_toggle(
            "freelook",
            "Free look",
            Rc::new(move || free_read.read().free_look),
            Rc::new(move |value| free_write.write(&|settings| settings.free_look = value)),
        ),
    ]
}

/// Bind controller vibration rows.
pub fn bind_controller_vibration(service: VibrationService) -> Vec<SettingBinding> {
    let toggle_read = Rc::clone(&service);
    let toggle_write = Rc::clone(&service);
    let strength_read = Rc::clone(&service);
    let strength_write = Rc::clone(&service);
    vec![
        input_toggle(
            "controller-vibration",
            "Controller vibration",
            Rc::new(move || toggle_read.read().controller_vibration),
            Rc::new(move |value| {
                toggle_write.write(&|settings| settings.controller_vibration = value);
            }),
        ),
        input_numeric(
            "controller-vibration-strength",
            "Vibration strength",
            0.0,
            1.0,
            0.05,
            Rc::new(move || strength_read.read().controller_vibration_strength),
            Rc::new(move |value| {
                strength_write.write(&|settings| settings.controller_vibration_strength = value);
            }),
        ),
    ]
}

/// Output sample format: rate in Hz, bits per sample, channel count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioOutputFormat {
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Bits per sample (8 or 16).
    pub sample_bits: u8,
    /// Channel count (1 or 2).
    pub channels: u8,
}

/// Default output format: 44100 Hz stereo 16-bit.
pub const DEFAULT_AUDIO_OUTPUT_FORMAT: AudioOutputFormat = AudioOutputFormat {
    sample_rate: 44100,
    sample_bits: 16,
    channels: 2,
};

/// Offered output sample rates in Hz.
#[must_use]
pub fn audio_output_rates() -> Vec<u32> {
    vec![11025, 22050, 44100, 48000]
}

/// Validate an output format: 8000-192000 Hz, 1 or 2 channels, 8 or 16 bits.
pub fn audio_output_format_validate(format: &AudioOutputFormat) -> Result<AudioOutputFormat, ClientError> {
    if !(8000..=192000).contains(&format.sample_rate)
        || (format.channels != 1 && format.channels != 2)
        || (format.sample_bits != 8 && format.sample_bits != 16)
    {
        return Err(ClientError::BadUi(
            "Audio output requires 8000-192000 Hz, 1 or 2 channels, and 8 or 16 bits".to_string(),
        ));
    }
    Ok(*format)
}

/// Read/select surface for the output sample format, when the backend
/// exposes one.
pub trait AudioOutputFormatControl {
    /// Read the current format.
    fn read(&self) -> AudioOutputFormat;
    /// Select a new format.
    fn select(&self, format: &AudioOutputFormat);
}

/// Output-device surface behind the audio settings rows.
pub trait AudioOutputSettings {
    /// Selected device name, or `None` for the system default.
    fn selected(&self) -> Option<String>;
    /// Known device names.
    fn devices(&self) -> Vec<String>;
    /// Select a device, or `None` for the system default.
    fn select(&self, name: Option<&str>);
    /// Report a selection failure to the operator.
    fn report(&self, message: &str);
    /// Format control, when the backend exposes one.
    fn format(&self) -> Option<Rc<dyn AudioOutputFormatControl>>;
}

/// Bind output device, format, and volume rows.
pub fn bind_audio_settings(
    service: Rc<dyn SettingsValueService<AudioSettings>>,
    output: Option<Rc<dyn AudioOutputSettings>>,
) -> Vec<SettingBinding> {
    let mut settings: Vec<SettingBinding> = Vec::new();
    if let Some(output) = output.clone() {
        let read_output = Rc::clone(&output);
        let choices_output = Rc::clone(&output);
        let write_output = Rc::clone(&output);
        settings.push(SettingBinding {
            id: control_id("ui:audio:device"),
            label: "Output device".to_string(),
            category: SettingCategory::Audio,
            enabled: Rc::new(|| true),
            kind: SettingBindingKind::Choice {
                read: Rc::new(move || match read_output.selected() {
                    None => "default".to_string(),
                    Some(name) => format!("device:{name}"),
                }),
                choices: Rc::new(move || {
                    let mut names: HashSet<String> = choices_output.devices().into_iter().collect();
                    if let Some(current) = choices_output.selected() {
                        names.insert(current);
                    }
                    let mut rows = vec![UiChoice {
                        id: "default".to_string(),
                        label: "System default".to_string(),
                    }];
                    let mut devices: Vec<String> = names.into_iter().collect();
                    devices.sort();
                    for name in devices {
                        rows.push(UiChoice {
                            id: format!("device:{name}"),
                            label: name,
                        });
                    }
                    rows
                }),
                write: Rc::new(move |value| {
                    if value != "default" && !value.starts_with("device:") {
                        write_output.report("Audio output selection failed: Unknown audio output choice");
                        return;
                    }
                    write_output.select(if value == "default" {
                        None
                    } else {
                        Some(&value["device:".len()..])
                    });
                }),
            },
        });
        if let Some(format) = output.format() {
            for (id, label, field) in [
                ("ui:audio:rate", "Output sample rate", "sample_rate"),
                ("ui:audio:bits", "Output sample bits", "sample_bits"),
                ("ui:audio:channels", "Output channels", "channels"),
            ] {
                let values: Vec<u32> = match field {
                    "sample_rate" => audio_output_rates(),
                    "sample_bits" => vec![8, 16],
                    _ => vec![1, 2],
                };
                let read_format = Rc::clone(&format);
                let choices_format = Rc::clone(&format);
                let write_format = Rc::clone(&format);
                let report_output = Rc::clone(&output);
                settings.push(SettingBinding {
                    id: control_id(id),
                    label: label.to_string(),
                    category: SettingCategory::Audio,
                    enabled: Rc::new(|| true),
                    kind: SettingBindingKind::Choice {
                        read: Rc::new(move || {
                            let current = read_format.read();
                            match field {
                                "sample_rate" => current.sample_rate.to_string(),
                                "sample_bits" => current.sample_bits.to_string(),
                                _ => current.channels.to_string(),
                            }
                        }),
                        choices: Rc::new(move || {
                            let current = choices_format.read();
                            let selected = match field {
                                "sample_rate" => current.sample_rate,
                                "sample_bits" => u32::from(current.sample_bits),
                                _ => u32::from(current.channels),
                            };
                            let mut merged: HashSet<u32> = values.iter().copied().collect();
                            merged.insert(selected);
                            let mut sorted: Vec<u32> = merged.into_iter().collect();
                            sorted.sort_unstable();
                            sorted
                                .into_iter()
                                .map(|value| UiChoice {
                                    id: value.to_string(),
                                    label: match field {
                                        "sample_rate" => format!("{value} Hz"),
                                        "sample_bits" => format!("{value}-bit"),
                                        _ => {
                                            if value == 1 {
                                                "Mono".to_string()
                                            } else {
                                                "Stereo".to_string()
                                            }
                                        }
                                    },
                                })
                                .collect()
                        }),
                        write: Rc::new(move |value| {
                            let parsed: u32 = value.parse().unwrap_or(0);
                            let mut next = write_format.read();
                            match field {
                                "sample_rate" => next.sample_rate = parsed,
                                "sample_bits" => next.sample_bits = parsed.min(255) as u8,
                                _ => next.channels = parsed.min(255) as u8,
                            }
                            match audio_output_format_validate(&next) {
                                Ok(valid) => write_format.select(&valid),
                                Err(error) => report_output.report(&format!("Audio format selection failed: {error}")),
                            }
                        }),
                    },
                });
            }
        }
    }
    let effects_read = Rc::clone(&service);
    let effects_write = Rc::clone(&service);
    let music_read = Rc::clone(&service);
    let music_write = Rc::clone(&service);
    settings.push(SettingBinding {
        id: control_id("ui:audio:effects"),
        label: "Effects volume".to_string(),
        category: SettingCategory::Audio,
        enabled: Rc::new(|| true),
        kind: SettingBindingKind::Slider {
            read: Rc::new(move || effects_read.read().effects_volume),
            write: Rc::new(move |value| {
                effects_write.write(&|settings| settings.effects_volume = value);
            }),
            minimum: 0.0,
            maximum: 1.0,
            step: 0.05,
            format_value: None,
        },
    });
    settings.push(SettingBinding {
        id: control_id("ui:audio:music"),
        label: "Music volume".to_string(),
        category: SettingCategory::Audio,
        enabled: Rc::new(|| true),
        kind: SettingBindingKind::Slider {
            read: Rc::new(move || music_read.read().music_volume),
            write: Rc::new(move |value| {
                music_write.write(&|settings| settings.music_volume = value);
            }),
            minimum: 0.0,
            maximum: 1.0,
            step: 0.05,
            format_value: None,
        },
    });
    settings
}

/// Bind the Quake II music shuffle toggle plus the menu-track choice when a
/// track lister is available. A missing registry binds nothing.
pub fn bind_music_playlist_settings(
    registry: Option<&Rc<dyn SettingCvars>>,
    tracks: Option<Rc<dyn Fn() -> Vec<String>>>,
) -> Vec<SettingBinding> {
    let Some(registry) = registry.cloned() else {
        return Vec::new();
    };
    let shuffle_read = Rc::clone(&registry);
    let shuffle_write = Rc::clone(&registry);
    let mut settings = vec![SettingBinding {
        id: control_id("ui:audio:shuffle"),
        label: "Shuffle Quake II gameplay music".to_string(),
        category: SettingCategory::Audio,
        enabled: Rc::new(|| true),
        kind: SettingBindingKind::Toggle {
            read: Rc::new(move || shuffle_read.variable_value("music_shuffle") != 0.0),
            write: Rc::new(move |value| {
                shuffle_write.set("music_shuffle", if value { "1" } else { "0" });
            }),
        },
    }];
    if let Some(tracks) = tracks {
        let read_registry = Rc::clone(&registry);
        let write_registry = Rc::clone(&registry);
        let choices_registry = Rc::clone(&registry);
        settings.push(SettingBinding {
            id: control_id("ui:audio:menu-track"),
            label: "Menu music".to_string(),
            category: SettingCategory::Audio,
            enabled: Rc::new(|| true),
            kind: SettingBindingKind::Choice {
                read: Rc::new(move || {
                    read_registry
                        .find("music_menu_track")
                        .map(|view| view.value)
                        .unwrap_or_else(|| "auto".to_string())
                }),
                write: Rc::new(move |value| write_registry.set("music_menu_track", value)),
                choices: Rc::new(move || {
                    let current = choices_registry
                        .find("music_menu_track")
                        .map(|view| view.value)
                        .unwrap_or_else(|| "auto".to_string());
                    let mut names: HashSet<String> = tracks().into_iter().collect();
                    if current != "auto" && current != "0" {
                        names.insert(current);
                    }
                    let mut rows = vec![
                        UiChoice {
                            id: "auto".to_string(),
                            label: "Automatic".to_string(),
                        },
                        UiChoice {
                            id: "0".to_string(),
                            label: "Off".to_string(),
                        },
                    ];
                    let mut listed: Vec<String> = names.into_iter().collect();
                    listed.sort();
                    for name in listed {
                        rows.push(UiChoice {
                            id: name.clone(),
                            label: name,
                        });
                    }
                    rows
                }),
            },
        });
    }
    settings
}

/// Bind the geometry sound-obstruction toggle. A missing registry binds
/// nothing.
pub fn bind_audio_geometry_settings(registry: Option<&Rc<dyn SettingCvars>>) -> Vec<SettingBinding> {
    let Some(registry) = registry.cloned() else {
        return Vec::new();
    };
    let read_registry = Rc::clone(&registry);
    let write_registry = Rc::clone(&registry);
    vec![SettingBinding {
        id: control_id("ui:audio:geometry"),
        label: "Geometry sound obstruction".to_string(),
        category: SettingCategory::Audio,
        enabled: Rc::new(|| true),
        kind: SettingBindingKind::Toggle {
            read: Rc::new(move || read_registry.variable_value("s_geometryAcoustics") != 0.0),
            write: Rc::new(move |value| {
                write_registry.set("s_geometryAcoustics", if value { "1" } else { "0" });
            }),
        },
    }]
}

/// Stick response curve: radial (Ironwail-style) or axial (Q2-style).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StickCurve {
    /// Radial deadzone with an outer threshold.
    Radial {
        /// Deadzone radius.
        deadzone: f32,
        /// Outer threshold.
        outer_threshold: f32,
        /// Response exponent.
        exponent: f32,
    },
    /// Per-axis deadzone.
    Axial {
        /// Deadzone radius.
        deadzone: f32,
        /// Response exponent.
        exponent: f32,
    },
}

impl StickCurve {
    /// Donor curve kind name.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            StickCurve::Radial { .. } => "radial",
            StickCurve::Axial { .. } => "axial",
        }
    }

    /// Deadzone radius.
    #[must_use]
    pub fn deadzone(&self) -> f32 {
        match self {
            StickCurve::Radial { deadzone, .. } | StickCurve::Axial { deadzone, .. } => *deadzone,
        }
    }

    /// Response exponent.
    #[must_use]
    pub fn exponent(&self) -> f32 {
        match self {
            StickCurve::Radial { exponent, .. } | StickCurve::Axial { exponent, .. } => *exponent,
        }
    }
}

/// Gyro yaw source axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GyroYawAxis {
    /// Y axis.
    Y,
    /// Z axis.
    Z,
}

/// Gyro tuning carried with the gamepad tuning object.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GyroTuningView {
    /// Gyro enabled.
    pub enabled: bool,
    /// Yaw sensitivity.
    pub yaw_sensitivity: f32,
    /// Pitch sensitivity.
    pub pitch_sensitivity: f32,
    /// Yaw source axis.
    pub yaw_axis: GyroYawAxis,
}

/// Full gamepad tuning object sampled by user-command builders.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GamepadTuningView {
    /// Move-stick curve.
    pub move_stick: StickCurve,
    /// Look-stick curve.
    pub look_stick: StickCurve,
    /// Swap the move and look sticks.
    pub swap_sticks: bool,
    /// Turn speed in degrees per second.
    pub yaw_degrees_per_second: f32,
    /// Look speed in degrees per second.
    pub pitch_degrees_per_second: f32,
    /// Invert the controller pitch axis.
    pub invert_pitch: bool,
    /// Forward sensitivity.
    pub forward_sensitivity: f32,
    /// Side sensitivity.
    pub side_sensitivity: f32,
    /// Trigger threshold.
    pub trigger_threshold: f32,
    /// Gyro tuning.
    pub gyro: GyroTuningView,
}

/// Default gamepad tuning, ported from the donor gamepad input module.
pub const DEFAULT_GAMEPAD_TUNING: GamepadTuningView = GamepadTuningView {
    move_stick: StickCurve::Radial {
        deadzone: 0.175,
        outer_threshold: 0.02,
        exponent: 2.0,
    },
    look_stick: StickCurve::Radial {
        deadzone: 0.175,
        outer_threshold: 0.02,
        exponent: 2.0,
    },
    swap_sticks: false,
    yaw_degrees_per_second: 240.0,
    pitch_degrees_per_second: 130.0,
    invert_pitch: false,
    forward_sensitivity: 1.0,
    side_sensitivity: 1.0,
    trigger_threshold: 0.2,
    gyro: GyroTuningView {
        enabled: false,
        yaw_sensitivity: 1.0,
        pitch_sensitivity: 1.0,
        yaw_axis: GyroYawAxis::Y,
    },
};

/// Validate a gamepad tuning object, mirroring the donor range checks.
pub fn validate_gamepad_tuning(tuning: &GamepadTuningView) -> Result<GamepadTuningView, ClientError> {
    for curve in [tuning.move_stick, tuning.look_stick] {
        let deadzone_ok = curve.deadzone().is_finite() && curve.deadzone() >= 0.0 && curve.deadzone() < 1.0;
        let exponent_ok = curve.exponent().is_finite() && curve.exponent() > 0.0;
        let outer_ok = match curve {
            StickCurve::Axial { .. } => true,
            StickCurve::Radial {
                outer_threshold,
                deadzone,
                ..
            } => outer_threshold.is_finite() && outer_threshold >= 0.0 && deadzone + outer_threshold < 1.0,
        };
        if !deadzone_ok || !exponent_ok || !outer_ok {
            return Err(ClientError::BadUi(
                "Invalid gamepad deadzone or response curve".to_string(),
            ));
        }
    }
    for value in [
        tuning.yaw_degrees_per_second,
        tuning.pitch_degrees_per_second,
        tuning.forward_sensitivity,
        tuning.side_sensitivity,
        tuning.gyro.yaw_sensitivity,
        tuning.gyro.pitch_sensitivity,
    ] {
        if !value.is_finite() {
            return Err(ClientError::BadUi("Invalid gamepad sensitivity".to_string()));
        }
    }
    if !tuning.trigger_threshold.is_finite() || tuning.trigger_threshold < 0.0 || tuning.trigger_threshold > 1.0 {
        return Err(ClientError::BadUi("Invalid trigger threshold".to_string()));
    }
    Ok(*tuning)
}

/// Live raw and curved deflection of one stick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StickPreview {
    /// Raw deflection.
    pub raw: Vec2,
    /// Curved deflection.
    pub curved: Vec2,
}

/// Live stick preview sampled from the gamepad input.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GamepadPreview {
    /// Move-stick preview.
    pub move_stick: StickPreview,
    /// Look-stick preview.
    pub look_stick: StickPreview,
}

/// Host owning the tuning objects sampled by the next user command.
///
/// These controls mutate the objects the next real user command samples.
pub trait InputTuningHost {
    /// Read the mouse tuning object.
    fn mouse_tuning(&self) -> MouseTuningView;
    /// Replace the mouse tuning object.
    fn set_mouse_tuning(&self, tuning: &MouseTuningView);
    /// Read always-run.
    fn always_run(&self) -> bool;
    /// Write always-run.
    fn set_always_run(&self, value: bool);
    /// Command-builder dialect (gates Q1-only look spring).
    fn builder_dialect(&self) -> CommandDialect;
    /// Read the gamepad tuning object.
    fn gamepad_tuning(&self) -> GamepadTuningView;
    /// Replace the gamepad tuning object.
    fn set_gamepad_tuning(&self, tuning: &GamepadTuningView);
    /// Sample the live stick preview.
    fn gamepad_preview(&self) -> GamepadPreview;
}

struct HostPrimaryInputService {
    host: Rc<RefCell<dyn InputTuningHost>>,
}

impl SettingsValueService<PrimaryInputSettings> for HostPrimaryInputService {
    fn read(&self) -> PrimaryInputSettings {
        let mouse = self.host.borrow().mouse_tuning();
        PrimaryInputSettings {
            sensitivity: mouse.sensitivity,
            pitch: mouse.pitch,
            yaw: mouse.yaw,
            invert_mouse: mouse.invert_pitch,
            always_run: self.host.borrow().always_run(),
        }
    }

    fn write(&self, update: &dyn Fn(&mut PrimaryInputSettings)) {
        let mut current = self.read();
        update(&mut current);
        let mut mouse = self.host.borrow().mouse_tuning();
        mouse.sensitivity = current.sensitivity;
        mouse.pitch = current.pitch;
        mouse.yaw = current.yaw;
        mouse.invert_pitch = current.invert_mouse;
        self.host.borrow().set_mouse_tuning(&mouse);
        self.host.borrow().set_always_run(current.always_run);
    }
}

struct HostMouseMotionService {
    host: Rc<RefCell<dyn InputTuningHost>>,
}

impl SettingsValueService<MouseMotionSettings> for HostMouseMotionService {
    fn read(&self) -> MouseMotionSettings {
        let mouse = self.host.borrow().mouse_tuning();
        MouseMotionSettings {
            acceleration: mouse.acceleration,
            filter: mouse.filter,
            free_look: mouse.free_look,
            look_spring: mouse.look_spring,
            look_strafe: mouse.look_strafe,
        }
    }

    fn write(&self, update: &dyn Fn(&mut MouseMotionSettings)) {
        let mut current = self.read();
        update(&mut current);
        let mut mouse = self.host.borrow().mouse_tuning();
        mouse.acceleration = current.acceleration;
        mouse.filter = current.filter;
        mouse.free_look = current.free_look;
        mouse.look_spring = current.look_spring;
        mouse.look_strafe = current.look_strafe;
        self.host.borrow().set_mouse_tuning(&mouse);
    }
}

/// Bind vibration, primary mouse, mouse motion, and gamepad rows against one
/// tuning host.
pub fn bind_input_settings(
    host: &Rc<RefCell<dyn InputTuningHost>>,
    vibration: Option<VibrationService>,
) -> Vec<SettingBinding> {
    let mut settings = Vec::new();
    if let Some(vibration) = vibration {
        settings.extend(bind_controller_vibration(vibration));
    }
    let primary: Rc<dyn SettingsValueService<PrimaryInputSettings>> =
        Rc::new(HostPrimaryInputService { host: Rc::clone(host) });
    settings.extend(bind_primary_input_settings(primary));
    let motion: Rc<dyn SettingsValueService<MouseMotionSettings>> =
        Rc::new(HostMouseMotionService { host: Rc::clone(host) });
    let spring_host = Rc::clone(host);
    let spring_available: Rc<dyn Fn() -> bool> = Rc::new(move || {
        matches!(
            spring_host.borrow().builder_dialect(),
            CommandDialect::Q1Netquake | CommandDialect::Q1Quakeworld
        )
    });
    settings.extend(bind_mouse_motion_settings(motion, spring_available));
    settings.extend(bind_gamepad_settings(host));
    settings
}

/// One scalar gamepad row: ids, label, range, and tuning accessors.
type ScalarRow = (
    &'static str,
    &'static str,
    f32,
    f32,
    f32,
    fn(&GamepadTuningView) -> f32,
    fn(&mut GamepadTuningView, f32),
);

/// One response-curve row: ids, label, stick, and curve accessors.
type CurveRow = (
    &'static str,
    &'static str,
    GamepadStick,
    fn(&GamepadTuningView) -> f32,
    fn(&mut GamepadTuningView, StickCurve),
);

/// Which stick a gamepad row edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum GamepadStick {
    Move,
    Look,
}

impl GamepadStick {
    fn name(&self) -> &'static str {
        match self {
            GamepadStick::Move => "Move",
            GamepadStick::Look => "Look",
        }
    }

    fn id(&self) -> &'static str {
        match self {
            GamepadStick::Move => "move",
            GamepadStick::Look => "look",
        }
    }
}

/// Bind invert, swap, speed, deadzone, curve, trigger, sensitivity, and live
/// preview rows against one tuning host.
///
/// Every update re-validates the merged tuning; invalid merges panic,
/// mirroring the donor throw.
pub fn bind_gamepad_settings(host: &Rc<RefCell<dyn InputTuningHost>>) -> Vec<SettingBinding> {
    let update = |host: &Rc<RefCell<dyn InputTuningHost>>| {
        let host = Rc::clone(host);
        Rc::new(move |next: GamepadTuningView| match validate_gamepad_tuning(&next) {
            Ok(valid) => host.borrow().set_gamepad_tuning(&valid),
            Err(error) => panic!("{error}"),
        })
    };
    let update_tuning = update(host);
    let invert_read = Rc::clone(host);
    let invert_write = Rc::clone(host);
    let invert_update = Rc::clone(&update_tuning);
    let swap_read = Rc::clone(host);
    let swap_write = Rc::clone(host);
    let swap_update = Rc::clone(&update_tuning);
    let mut settings = vec![
        input_toggle(
            "invert-controller",
            "Invert controller",
            Rc::new(move || invert_read.borrow().gamepad_tuning().invert_pitch),
            Rc::new(move |value| {
                let mut next = invert_write.borrow().gamepad_tuning();
                next.invert_pitch = value;
                invert_update(next);
            }),
        ),
        input_toggle(
            "swap-sticks",
            "Swap controller sticks",
            Rc::new(move || swap_read.borrow().gamepad_tuning().swap_sticks),
            Rc::new(move |value| {
                let mut next = swap_write.borrow().gamepad_tuning();
                next.swap_sticks = value;
                swap_update(next);
            }),
        ),
    ];
    let speeds: [ScalarRow; 2] = [
        (
            "look-speed",
            "Controller turn speed",
            30.0,
            720.0,
            10.0,
            |tuning: &GamepadTuningView| tuning.yaw_degrees_per_second,
            |tuning: &mut GamepadTuningView, value: f32| tuning.yaw_degrees_per_second = value,
        ),
        (
            "pitch-speed",
            "Controller look speed",
            30.0,
            720.0,
            10.0,
            |tuning: &GamepadTuningView| tuning.pitch_degrees_per_second,
            |tuning: &mut GamepadTuningView, value: f32| tuning.pitch_degrees_per_second = value,
        ),
    ];
    for (id, label, minimum, maximum, step, get, set) in speeds {
        let read_host = Rc::clone(host);
        let write_host = Rc::clone(host);
        let write_update = Rc::clone(&update_tuning);
        settings.push(input_numeric(
            id,
            label,
            minimum,
            maximum,
            step,
            Rc::new(move || get(&read_host.borrow().gamepad_tuning())),
            Rc::new(move |value| {
                let mut next = write_host.borrow().gamepad_tuning();
                set(&mut next, value);
                write_update(next);
            }),
        ));
    }
    for stick in [GamepadStick::Move, GamepadStick::Look] {
        let read_host = Rc::clone(host);
        let write_host = Rc::clone(host);
        let write_update = Rc::clone(&update_tuning);
        let label = format!("{} stick deadzone", stick.name());
        let id = format!("{}-deadzone", stick.id());
        settings.push(input_numeric(
            &id,
            &label,
            0.0,
            0.5,
            0.01,
            Rc::new(move || {
                let tuning = read_host.borrow().gamepad_tuning();
                match stick {
                    GamepadStick::Move => tuning.move_stick,
                    GamepadStick::Look => tuning.look_stick,
                }
                .deadzone()
            }),
            Rc::new(move |value| {
                let mut next = write_host.borrow().gamepad_tuning();
                let selected = match stick {
                    GamepadStick::Move => next.move_stick,
                    GamepadStick::Look => next.look_stick,
                };
                let curve = match selected {
                    StickCurve::Radial {
                        outer_threshold,
                        exponent,
                        ..
                    } => StickCurve::Radial {
                        deadzone: value.min(0.999 - outer_threshold),
                        outer_threshold,
                        exponent,
                    },
                    StickCurve::Axial { exponent, .. } => StickCurve::Axial {
                        deadzone: value,
                        exponent,
                    },
                };
                match stick {
                    GamepadStick::Move => next.move_stick = curve,
                    GamepadStick::Look => next.look_stick = curve,
                }
                write_update(next);
            }),
        ));
    }
    let curves: [CurveRow; 2] = [
        (
            "look-curve",
            "Look response curve",
            GamepadStick::Look,
            |tuning: &GamepadTuningView| tuning.look_stick.exponent(),
            |tuning: &mut GamepadTuningView, curve: StickCurve| tuning.look_stick = curve,
        ),
        (
            "move-curve",
            "Move response curve",
            GamepadStick::Move,
            |tuning: &GamepadTuningView| tuning.move_stick.exponent(),
            |tuning: &mut GamepadTuningView, curve: StickCurve| tuning.move_stick = curve,
        ),
    ];
    for (id, label, stick, get, set) in curves {
        let read_host = Rc::clone(host);
        let write_host = Rc::clone(host);
        let write_update = Rc::clone(&update_tuning);
        settings.push(input_numeric(
            id,
            label,
            0.5,
            4.0,
            0.1,
            Rc::new(move || get(&read_host.borrow().gamepad_tuning())),
            Rc::new(move |value| {
                let mut next = write_host.borrow().gamepad_tuning();
                let selected = match stick {
                    GamepadStick::Move => next.move_stick,
                    GamepadStick::Look => next.look_stick,
                };
                let curve = match selected {
                    StickCurve::Radial {
                        deadzone,
                        outer_threshold,
                        ..
                    } => StickCurve::Radial {
                        deadzone,
                        outer_threshold,
                        exponent: value,
                    },
                    StickCurve::Axial { deadzone, .. } => StickCurve::Axial {
                        deadzone,
                        exponent: value,
                    },
                };
                set(&mut next, curve);
                write_update(next);
            }),
        ));
    }
    let scalars: [ScalarRow; 3] = [
        (
            "trigger",
            "Trigger threshold",
            0.05,
            0.95,
            0.05,
            |tuning: &GamepadTuningView| tuning.trigger_threshold,
            |tuning: &mut GamepadTuningView, value: f32| tuning.trigger_threshold = value,
        ),
        (
            "forward-sensitivity",
            "Forward controller sensitivity",
            0.0,
            3.0,
            0.05,
            |tuning: &GamepadTuningView| tuning.forward_sensitivity,
            |tuning: &mut GamepadTuningView, value: f32| tuning.forward_sensitivity = value,
        ),
        (
            "side-sensitivity",
            "Side controller sensitivity",
            0.0,
            3.0,
            0.05,
            |tuning: &GamepadTuningView| tuning.side_sensitivity,
            |tuning: &mut GamepadTuningView, value: f32| tuning.side_sensitivity = value,
        ),
    ];
    for (id, label, minimum, maximum, step, get, set) in scalars {
        let read_host = Rc::clone(host);
        let write_host = Rc::clone(host);
        let write_update = Rc::clone(&update_tuning);
        settings.push(input_numeric(
            id,
            label,
            minimum,
            maximum,
            step,
            Rc::new(move || get(&read_host.borrow().gamepad_tuning())),
            Rc::new(move |value| {
                let mut next = write_host.borrow().gamepad_tuning();
                set(&mut next, value);
                write_update(next);
            }),
        ));
    }
    for stick in [GamepadStick::Move, GamepadStick::Look] {
        let read_host = Rc::clone(host);
        let write_host = Rc::clone(host);
        let write_update = Rc::clone(&update_tuning);
        settings.push(SettingBinding {
            id: control_id(&format!("ui:input:{}-curve-type", stick.id())),
            label: format!("{} deadzone shape", stick.name()),
            category: SettingCategory::Input,
            enabled: Rc::new(|| true),
            kind: SettingBindingKind::Choice {
                read: Rc::new(move || {
                    let tuning = read_host.borrow().gamepad_tuning();
                    match stick {
                        GamepadStick::Move => tuning.move_stick,
                        GamepadStick::Look => tuning.look_stick,
                    }
                    .kind()
                    .to_string()
                }),
                choices: Rc::new(|| {
                    vec![
                        UiChoice {
                            id: "radial".to_string(),
                            label: "Radial".to_string(),
                        },
                        UiChoice {
                            id: "axial".to_string(),
                            label: "Axial".to_string(),
                        },
                    ]
                }),
                write: Rc::new(move |value| {
                    if value != "radial" && value != "axial" {
                        panic!("Unknown controller curve");
                    }
                    let mut next = write_host.borrow().gamepad_tuning();
                    let previous = match stick {
                        GamepadStick::Move => next.move_stick,
                        GamepadStick::Look => next.look_stick,
                    };
                    let curve = if value == "radial" {
                        StickCurve::Radial {
                            deadzone: previous.deadzone(),
                            exponent: previous.exponent(),
                            outer_threshold: 0.02_f32.min((1.0 - previous.deadzone()) / 2.0),
                        }
                    } else {
                        StickCurve::Axial {
                            deadzone: previous.deadzone(),
                            exponent: previous.exponent(),
                        }
                    };
                    match stick {
                        GamepadStick::Move => next.move_stick = curve,
                        GamepadStick::Look => next.look_stick = curve,
                    }
                    write_update(next);
                }),
            },
        });
        let outer_read = Rc::clone(host);
        let outer_write = Rc::clone(host);
        let outer_update = Rc::clone(&update_tuning);
        let outer_enabled = Rc::clone(host);
        let mut outer = input_numeric(
            &format!("{}-outer", stick.id()),
            &format!("{} outer threshold", stick.name()),
            0.0,
            0.49,
            0.01,
            Rc::new(move || {
                match match stick {
                    GamepadStick::Move => outer_read.borrow().gamepad_tuning().move_stick,
                    GamepadStick::Look => outer_read.borrow().gamepad_tuning().look_stick,
                } {
                    StickCurve::Radial { outer_threshold, .. } => outer_threshold,
                    StickCurve::Axial { .. } => 0.0,
                }
            }),
            Rc::new(move |value| {
                let mut next = outer_write.borrow().gamepad_tuning();
                let selected = match stick {
                    GamepadStick::Move => next.move_stick,
                    GamepadStick::Look => next.look_stick,
                };
                if let StickCurve::Radial { deadzone, exponent, .. } = selected {
                    let curve = StickCurve::Radial {
                        deadzone,
                        exponent,
                        outer_threshold: value.min(0.999 - deadzone),
                    };
                    match stick {
                        GamepadStick::Move => next.move_stick = curve,
                        GamepadStick::Look => next.look_stick = curve,
                    }
                    outer_update(next);
                }
            }),
        );
        outer.enabled = Rc::new(move || {
            matches!(
                match stick {
                    GamepadStick::Move => outer_enabled.borrow().gamepad_tuning().move_stick,
                    GamepadStick::Look => outer_enabled.borrow().gamepad_tuning().look_stick,
                },
                StickCurve::Radial { .. }
            )
        });
        settings.push(outer);
        for component in ['x', 'y'] {
            let preview_host = Rc::clone(host);
            let format_host = Rc::clone(host);
            let mut preview = input_numeric(
                &format!("{}-preview-{component}", stick.id()),
                &format!("{} {} live", stick.name(), component.to_ascii_uppercase()),
                -1.0,
                1.0,
                0.01,
                Rc::new(move || {
                    let preview = preview_host.borrow().gamepad_preview();
                    let selected = match stick {
                        GamepadStick::Move => preview.move_stick,
                        GamepadStick::Look => preview.look_stick,
                    };
                    if component == 'x' {
                        selected.curved.x
                    } else {
                        selected.curved.y
                    }
                }),
                Rc::new(|_| {}),
            );
            preview.enabled = Rc::new(|| false);
            if let SettingBindingKind::Slider { format_value, .. } = &mut preview.kind {
                *format_value = Some(Rc::new(move |value| {
                    let preview = format_host.borrow().gamepad_preview();
                    let selected = match stick {
                        GamepadStick::Move => preview.move_stick,
                        GamepadStick::Look => preview.look_stick,
                    };
                    let raw = if component == 'x' {
                        selected.raw.x
                    } else {
                        selected.raw.y
                    };
                    format!("Raw {raw:.2} / {value:.2}")
                }));
            }
            settings.push(preview);
        }
    }
    settings
}

/// Numeric preference accessor.
type PreferenceNumber = Rc<dyn Fn(&UiPreferenceValues) -> f32>;
/// Numeric preference mutation.
type PreferenceNumberMut = Rc<dyn Fn(&mut UiPreferenceValues, f32)>;
/// Toggle preference accessor.
type PreferenceFlag = Rc<dyn Fn(&UiPreferenceValues) -> bool>;
/// Toggle preference mutation.
type PreferenceFlagMut = Rc<dyn Fn(&mut UiPreferenceValues, bool)>;

/// Per-seat UI preferences consumed directly by menu and HUD draws.
///
/// Values live in cvars (via the `accessibility` sibling port) when a
/// registry is attached, or in a local fallback otherwise. Callers hold an
/// `Rc` so bindings can borrow the preferences.
pub struct SeatUiPreferences {
    seat: SeatId,
    cvars: Option<Rc<dyn SettingCvars>>,
    local: RefCell<UiPreferenceValues>,
}

impl SeatUiPreferences {
    /// Build preferences for one seat, optionally backed by cvars.
    pub fn new(seat: SeatId, cvars: Option<Rc<dyn SettingCvars>>) -> Self {
        Self {
            seat,
            cvars,
            local: RefCell::new(DEFAULT_UI_PREFERENCES),
        }
    }

    /// Owning seat.
    #[must_use]
    pub fn seat(&self) -> SeatId {
        self.seat.clone()
    }

    /// Read the current values.
    #[must_use]
    pub fn values(&self) -> UiPreferenceValues {
        match &self.cvars {
            None => *self.local.borrow(),
            Some(cvars) => read_ui_preferences(cvars.as_ref(), self.seat.index()),
        }
    }

    /// Replace the current values.
    pub fn set_values(&self, values: &UiPreferenceValues) {
        match &self.cvars {
            None => *self.local.borrow_mut() = *values,
            Some(cvars) => write_ui_preferences(cvars.as_ref(), self.seat.index(), values),
        }
    }

    /// Bind every preference row.
    pub fn bindings(self: &Rc<Self>) -> Vec<SettingBinding> {
        let number = |key: &str,
                      label: &str,
                      minimum: f32,
                      maximum: f32,
                      step: f32,
                      get: PreferenceNumber,
                      set: PreferenceNumberMut| {
            let owner = Rc::clone(self);
            let read_owner = Rc::clone(self);
            SettingBinding {
                id: control_id(&format!("ui:accessibility:{key}")),
                label: label.to_string(),
                category: SettingCategory::Accessibility,
                enabled: Rc::new(|| true),
                kind: SettingBindingKind::Slider {
                    read: Rc::new(move || get(&read_owner.values())),
                    write: Rc::new(move |value| {
                        let mut current = owner.values();
                        set(&mut current, value);
                        owner.set_values(&current);
                    }),
                    minimum,
                    maximum,
                    step,
                    format_value: None,
                },
            }
        };
        let boolean = |key: &str, label: &str, get: PreferenceFlag, set: PreferenceFlagMut| {
            let owner = Rc::clone(self);
            let read_owner = Rc::clone(self);
            SettingBinding {
                id: control_id(&format!("ui:accessibility:{key}")),
                label: label.to_string(),
                category: SettingCategory::Accessibility,
                enabled: Rc::new(|| true),
                kind: SettingBindingKind::Toggle {
                    read: Rc::new(move || get(&read_owner.values())),
                    write: Rc::new(move |value| {
                        let mut current = owner.values();
                        set(&mut current, value);
                        owner.set_values(&current);
                    }),
                },
            }
        };
        let typeface_owner = Rc::clone(self);
        let typeface_read = Rc::clone(self);
        let typeface = SettingBinding {
            id: control_id("ui:accessibility:typeface"),
            label: "Typeface".to_string(),
            category: SettingCategory::Accessibility,
            enabled: Rc::new(|| true),
            kind: SettingBindingKind::Choice {
                read: Rc::new(move || {
                    match typeface_read.values().typeface {
                        Typeface::Standard => "standard",
                        Typeface::Bold => "bold",
                    }
                    .to_string()
                }),
                choices: Rc::new(|| {
                    vec![
                        UiChoice {
                            id: "standard".to_string(),
                            label: "Standard".to_string(),
                        },
                        UiChoice {
                            id: "bold".to_string(),
                            label: "Bold".to_string(),
                        },
                    ]
                }),
                write: Rc::new(move |value| {
                    let face = match value {
                        "standard" => Typeface::Standard,
                        "bold" => Typeface::Bold,
                        _ => panic!("Unknown typeface"),
                    };
                    let mut current = typeface_owner.values();
                    current.typeface = face;
                    typeface_owner.set_values(&current);
                }),
            },
        };
        let colors_owner = Rc::clone(self);
        let colors_read = Rc::clone(self);
        let color_mode = SettingBinding {
            id: control_id("ui:accessibility:colorMode"),
            label: "Interface colors".to_string(),
            category: SettingCategory::Accessibility,
            enabled: Rc::new(|| true),
            kind: SettingBindingKind::Choice {
                read: Rc::new(move || {
                    match colors_read.values().color_mode {
                        InterfaceColorMode::Standard => "standard",
                        InterfaceColorMode::BlueYellow => "blue-yellow",
                        InterfaceColorMode::Monochrome => "monochrome",
                    }
                    .to_string()
                }),
                choices: Rc::new(|| {
                    vec![
                        UiChoice {
                            id: "standard".to_string(),
                            label: "Standard".to_string(),
                        },
                        UiChoice {
                            id: "blue-yellow".to_string(),
                            label: "Blue and yellow".to_string(),
                        },
                        UiChoice {
                            id: "monochrome".to_string(),
                            label: "Monochrome".to_string(),
                        },
                    ]
                }),
                write: Rc::new(move |value| {
                    let mode = match value {
                        "standard" => InterfaceColorMode::Standard,
                        "blue-yellow" => InterfaceColorMode::BlueYellow,
                        "monochrome" => InterfaceColorMode::Monochrome,
                        _ => panic!("Unknown interface colors"),
                    };
                    let mut current = colors_owner.values();
                    current.color_mode = mode;
                    colors_owner.set_values(&current);
                }),
            },
        };
        let reset_owner = Rc::clone(self);
        vec![
            typeface,
            color_mode,
            number(
                "hudScale",
                "HUD size",
                0.5,
                1.5,
                0.05,
                Rc::new(|values| values.hud_scale),
                Rc::new(|values, value| values.hud_scale = value),
            ),
            number(
                "textScale",
                "Text size",
                0.75,
                2.0,
                0.05,
                Rc::new(|values| values.text_scale),
                Rc::new(|values, value| values.text_scale = value),
            ),
            number(
                "menuScale",
                "Menu size",
                0.75,
                1.0,
                0.05,
                Rc::new(|values| values.menu_scale),
                Rc::new(|values, value| values.menu_scale = value),
            ),
            number(
                "crosshairSize",
                "Crosshair size",
                2.0,
                32.0,
                1.0,
                Rc::new(|values| values.crosshair_size),
                Rc::new(|values, value| values.crosshair_size = value),
            ),
            boolean(
                "highContrast",
                "High contrast",
                Rc::new(|values| values.high_contrast),
                Rc::new(|values, value| values.high_contrast = value),
            ),
            boolean(
                "reducedFlashes",
                "Reduce HUD flashes",
                Rc::new(|values| values.reduced_flashes),
                Rc::new(|values, value| values.reduced_flashes = value),
            ),
            boolean(
                "captions",
                "Captions",
                Rc::new(|values| values.captions),
                Rc::new(|values, value| values.captions = value),
            ),
            boolean(
                "crosshair",
                "Crosshair",
                Rc::new(|values| values.crosshair),
                Rc::new(|values, value| values.crosshair = value),
            ),
            SettingBinding {
                id: control_id("ui:accessibility:reset"),
                label: "Reset accessibility settings".to_string(),
                category: SettingCategory::Accessibility,
                enabled: Rc::new(|| true),
                kind: SettingBindingKind::Button {
                    activate: Rc::new(move || {
                        reset_owner.set_values(&DEFAULT_UI_PREFERENCES);
                    }),
                },
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;

    use qa_core::identity::IdentityOwner;

    use super::llm::{
        LlmCatalog, LlmCatalogStatus, LlmCatalogs, LlmProvider, LlmProviderState, LlmProviders, LlmSnapshot,
        LlmSubscriptionAuth,
    };
    use super::*;
    use crate::ui::common::controller::headless_options;
    use crate::ui::types::{
        ContentId, DopplerSelection, EnvironmentSelection, PresentationSelection, ProviderRef, SeatInputFocus,
        SeatPresentationBinding, SeatUiController, UiDrawCommand, UiDrawContext,
    };

    struct Entry {
        value: String,
        latched_value: Option<String>,
        reset_value: String,
        flags: u32,
    }

    type Validator = Rc<dyn Fn(&str) -> Option<String>>;

    struct MemoryCvars {
        dialect: CommandDialect,
        vars: RefCell<HashMap<String, Entry>>,
        validators: RefCell<HashMap<String, Validator>>,
    }

    impl MemoryCvars {
        fn new(dialect: CommandDialect) -> Self {
            Self {
                dialect,
                vars: RefCell::new(HashMap::new()),
                validators: RefCell::new(HashMap::new()),
            }
        }

        fn with(mut self, name: &str, value: &str) -> Self {
            self.register(name, value, false);
            self
        }

        fn with_flags(mut self, name: &str, value: &str, flags: u32) -> Self {
            self.register(name, value, false);
            if let Some(entry) = self.vars.borrow_mut().get_mut(name) {
                entry.flags = flags;
            }
            self
        }

        fn with_latched(mut self, name: &str, value: &str, latched: &str) -> Self {
            self.register(name, value, false);
            if let Some(entry) = self.vars.borrow_mut().get_mut(name) {
                entry.latched_value = Some(latched.to_string());
            }
            self
        }

        fn get(&self, name: &str) -> Option<String> {
            self.vars.borrow().get(name).map(|entry| entry.value.clone())
        }
    }

    impl SettingCvars for MemoryCvars {
        fn dialect(&self) -> CommandDialect {
            self.dialect
        }

        fn find(&self, name: &str) -> Option<CvarView> {
            self.vars.borrow().get(name).map(|entry| CvarView {
                value: entry.value.clone(),
                latched_value: entry.latched_value.clone(),
                reset_value: entry.reset_value.clone(),
                flags: entry.flags,
            })
        }

        fn set(&self, name: &str, value: &str) {
            if let Some(validator) = self.validators.borrow().get(name) {
                if validator(value).is_some() {
                    return;
                }
            }
            self.vars
                .borrow_mut()
                .entry(name.to_string())
                .and_modify(|entry| entry.value = value.to_string())
                .or_insert(Entry {
                    value: value.to_string(),
                    latched_value: None,
                    reset_value: value.to_string(),
                    flags: 0,
                });
        }

        fn variable_value(&self, name: &str) -> f32 {
            self.vars
                .borrow()
                .get(name)
                .and_then(|entry| entry.value.parse::<f32>().ok())
                .unwrap_or(0.0)
        }
    }

    impl RegisterCvars for MemoryCvars {
        fn register(&mut self, name: &str, value: &str, archive: bool) {
            self.vars.borrow_mut().insert(
                name.to_string(),
                Entry {
                    value: value.to_string(),
                    latched_value: None,
                    reset_value: value.to_string(),
                    flags: u32::from(archive),
                },
            );
        }

        fn bind_validator(&mut self, name: &str, validate: CvarValidator) {
            self.validators.borrow_mut().insert(name.to_string(), validate);
        }
    }

    struct MemoryRestartSink {
        requests: RefCell<Vec<RestartKind>>,
    }

    impl RestartSink for MemoryRestartSink {
        fn request(&mut self, kind: RestartKind) {
            self.requests.borrow_mut().push(kind);
        }
    }

    struct MemoryService<T: Clone> {
        value: RefCell<T>,
    }

    impl<T: Clone> MemoryService<T> {
        fn new(value: T) -> Self {
            Self {
                value: RefCell::new(value),
            }
        }
    }

    impl<T: Clone> SettingsValueService<T> for MemoryService<T> {
        fn read(&self) -> T {
            self.value.borrow().clone()
        }

        fn write(&self, update: &dyn Fn(&mut T)) {
            update(&mut self.value.borrow_mut());
        }
    }

    struct MemoryHost {
        mouse: RefCell<MouseTuningView>,
        always_run: Cell<bool>,
        dialect: CommandDialect,
        gamepad: RefCell<GamepadTuningView>,
        preview: GamepadPreview,
    }

    impl MemoryHost {
        fn new(dialect: CommandDialect) -> Self {
            let still = StickPreview {
                raw: Vec2 { x: 0.0, y: 0.0 },
                curved: Vec2 { x: 0.0, y: 0.0 },
            };
            Self {
                mouse: RefCell::new(DEFAULT_MOUSE_TUNING),
                always_run: Cell::new(false),
                dialect,
                gamepad: RefCell::new(DEFAULT_GAMEPAD_TUNING),
                preview: GamepadPreview {
                    move_stick: still,
                    look_stick: still,
                },
            }
        }
    }

    impl InputTuningHost for MemoryHost {
        fn mouse_tuning(&self) -> MouseTuningView {
            *self.mouse.borrow()
        }

        fn set_mouse_tuning(&self, tuning: &MouseTuningView) {
            *self.mouse.borrow_mut() = *tuning;
        }

        fn always_run(&self) -> bool {
            self.always_run.get()
        }

        fn set_always_run(&self, value: bool) {
            self.always_run.set(value);
        }

        fn builder_dialect(&self) -> CommandDialect {
            self.dialect
        }

        fn gamepad_tuning(&self) -> GamepadTuningView {
            *self.gamepad.borrow()
        }

        fn set_gamepad_tuning(&self, tuning: &GamepadTuningView) {
            *self.gamepad.borrow_mut() = *tuning;
        }

        fn gamepad_preview(&self) -> GamepadPreview {
            self.preview
        }
    }

    struct MemoryFormat {
        format: Rc<RefCell<AudioOutputFormat>>,
    }

    impl AudioOutputFormatControl for MemoryFormat {
        fn read(&self) -> AudioOutputFormat {
            *self.format.borrow()
        }

        fn select(&self, format: &AudioOutputFormat) {
            *self.format.borrow_mut() = *format;
        }
    }

    struct MemoryOutput {
        selected: RefCell<Option<String>>,
        devices: Vec<String>,
        reports: RefCell<Vec<String>>,
        format: Option<Rc<MemoryFormat>>,
    }

    impl AudioOutputSettings for MemoryOutput {
        fn selected(&self) -> Option<String> {
            self.selected.borrow().clone()
        }

        fn devices(&self) -> Vec<String> {
            self.devices.clone()
        }

        fn select(&self, name: Option<&str>) {
            *self.selected.borrow_mut() = name.map(str::to_string);
        }

        fn report(&self, message: &str) {
            self.reports.borrow_mut().push(message.to_string());
        }

        fn format(&self) -> Option<Rc<dyn AudioOutputFormatControl>> {
            self.format
                .clone()
                .map(|format| format as Rc<dyn AudioOutputFormatControl>)
        }
    }

    struct FakeLlm;

    impl FakeLlm {
        fn snapshot() -> LlmSnapshot {
            let idle_provider = || LlmProviderState {
                configured: false,
                model: String::new(),
                base_url: String::new(),
            };
            let idle_catalog = || LlmCatalog {
                models: Vec::new(),
                status: LlmCatalogStatus::Idle,
            };
            LlmSnapshot {
                provider: LlmProvider::ChatGptApi,
                providers: LlmProviders {
                    subscription: idle_provider(),
                    api: idle_provider(),
                    other: idle_provider(),
                },
                catalogs: LlmCatalogs {
                    subscription: idle_catalog(),
                    api: idle_catalog(),
                    other: idle_catalog(),
                },
                subscription_auth: LlmSubscriptionAuth::Idle,
                reasoning_effort: None,
                errors: Vec::new(),
            }
        }
    }

    impl LlmSettingsUi for FakeLlm {
        fn read(&self) -> LlmSnapshot {
            Self::snapshot()
        }

        fn select_provider(&mut self, _provider: LlmProvider) -> Result<(), String> {
            Ok(())
        }

        fn set_model(&mut self, _provider: LlmProvider, _model: &str) -> Result<(), String> {
            Ok(())
        }

        fn save_api_key(&mut self, _provider: LlmProvider, _key: &str) -> Result<(), String> {
            Ok(())
        }

        fn remove_credential(&mut self, _provider: LlmProvider) -> Result<(), String> {
            Ok(())
        }

        fn save_other_service(&mut self, _base_url: &str, _model: &str) -> Result<(), String> {
            Ok(())
        }

        fn sign_in_subscription(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn cancel_sign_in(&mut self) {}

        fn refresh_models(&mut self, _provider: LlmProvider) -> Result<(), String> {
            Ok(())
        }

        fn set_reasoning_effort(&mut self, _provider: LlmProvider, _effort: Option<&str>) -> Result<(), String> {
            Ok(())
        }
    }

    fn owner() -> IdentityOwner {
        IdentityOwner::create("settings-test").expect("test owner")
    }

    fn spec(name: &str, kind: CvarSettingKind) -> CvarSettingSpec {
        CvarSettingSpec {
            name: name.to_string(),
            label: name.to_string(),
            category: SettingCategory::Audio,
            restart: None,
            kind,
        }
    }

    fn read_toggle(binding: &SettingBinding) -> bool {
        match &binding.kind {
            SettingBindingKind::Toggle { read, .. } => read(),
            _ => panic!("expected toggle"),
        }
    }

    fn write_toggle(binding: &SettingBinding, value: bool) {
        match &binding.kind {
            SettingBindingKind::Toggle { write, .. } => write(value),
            _ => panic!("expected toggle"),
        }
    }

    #[test]
    fn cvar_toggle_slider_choice_text_round_trip() {
        let cvars: Rc<dyn SettingCvars> = Rc::new(
            MemoryCvars::new(CommandDialect::Q3)
                .with("snd_on", "1")
                .with("snd_volume", "0.5")
                .with("snd_backend", "pulse")
                .with("player_name", "hero"),
        );
        let toggle = bind_cvar_setting(&cvars, spec("snd_on", CvarSettingKind::Toggle), None).unwrap();
        assert!(read_toggle(&toggle));
        write_toggle(&toggle, false);
        assert_eq!(cvars.find("snd_on").unwrap().value, "0");
        match bind_cvar_setting(&cvars, spec("snd_missing", CvarSettingKind::Toggle), None) {
            Ok(_) => panic!("expected missing-owner error"),
            Err(error) => assert!(error.to_string().contains("no owner"), "{error}"),
        }

        let slider = bind_cvar_setting(
            &cvars,
            spec(
                "snd_volume",
                CvarSettingKind::Slider {
                    minimum: 0.0,
                    maximum: 1.0,
                    step: 0.05,
                },
            ),
            None,
        )
        .unwrap();
        match &slider.kind {
            SettingBindingKind::Slider {
                read,
                write,
                minimum,
                maximum,
                step,
                ..
            } => {
                assert_eq!(read(), 0.5);
                assert_eq!((*minimum, *maximum, *step), (0.0, 1.0, 0.05));
                write(0.75);
            }
            _ => panic!("expected slider"),
        }
        assert_eq!(cvars.find("snd_volume").unwrap().value, "0.75");

        let choice = bind_cvar_setting(
            &cvars,
            spec(
                "snd_backend",
                CvarSettingKind::Choice {
                    choices: vec![
                        UiChoice {
                            id: "pulse".to_string(),
                            label: "Pulse".to_string(),
                        },
                        UiChoice {
                            id: "alsa".to_string(),
                            label: "ALSA".to_string(),
                        },
                    ],
                },
            ),
            None,
        )
        .unwrap();
        match &choice.kind {
            SettingBindingKind::Choice { read, write, choices } => {
                assert_eq!(read(), "pulse");
                assert_eq!(choices().len(), 2);
                write("alsa");
            }
            _ => panic!("expected choice"),
        }
        assert_eq!(cvars.find("snd_backend").unwrap().value, "alsa");

        let text = bind_cvar_setting(
            &cvars,
            spec(
                "player_name",
                CvarSettingKind::TextEntry {
                    maximum_length: 16,
                    submit_only: false,
                },
            ),
            None,
        )
        .unwrap();
        match &text.kind {
            SettingBindingKind::TextEntry {
                read,
                write,
                maximum_length,
                commit,
            } => {
                assert_eq!(read(), "hero");
                assert_eq!(*maximum_length, 16);
                assert!(commit.is_none());
                write("pro");
            }
            _ => panic!("expected text entry"),
        }
        assert_eq!(cvars.find("player_name").unwrap().value, "pro");
    }

    #[test]
    fn cvar_restart_requires_owner_and_requests_on_write() {
        let cvars: Rc<dyn SettingCvars> = Rc::new(MemoryCvars::new(CommandDialect::Q3).with("vid_mode", "3"));
        let spec = CvarSettingSpec {
            restart: Some(RestartKind::Video),
            ..spec(
                "vid_mode",
                CvarSettingKind::Slider {
                    minimum: 0.0,
                    maximum: 8.0,
                    step: 1.0,
                },
            )
        };
        match bind_cvar_setting(&cvars, spec.clone(), None) {
            Ok(_) => panic!("expected missing-restart-owner error"),
            Err(error) => assert!(error.to_string().contains("restart owner"), "{error}"),
        }
        let sink = Rc::new(RefCell::new(MemoryRestartSink {
            requests: RefCell::new(Vec::new()),
        }));
        let sink_dyn: Rc<RefCell<dyn RestartSink>> = sink.clone();
        let binding = bind_cvar_setting(&cvars, spec, Some(&sink_dyn)).unwrap();
        match &binding.kind {
            SettingBindingKind::Slider { write, .. } => write(4.0),
            _ => panic!("expected slider"),
        }
        assert_eq!(*sink.borrow().requests.borrow(), vec![RestartKind::Video]);
    }

    #[test]
    fn cvar_readonly_flags_disable_per_dialect() {
        let q3: Rc<dyn SettingCvars> = Rc::new(
            MemoryCvars::new(CommandDialect::Q3)
                .with_flags("locked", "1", CVAR_FLAG_READONLY)
                .with_flags("frozen", "1", CVAR_FLAG_INIT)
                .with("free", "1"),
        );
        assert!(!(bind_cvar_setting(&q3, spec("locked", CvarSettingKind::Toggle), None)
            .unwrap()
            .enabled)());
        assert!(!(bind_cvar_setting(&q3, spec("frozen", CvarSettingKind::Toggle), None)
            .unwrap()
            .enabled)());
        assert!((bind_cvar_setting(&q3, spec("free", CvarSettingKind::Toggle), None)
            .unwrap()
            .enabled)());

        let q2: Rc<dyn SettingCvars> = Rc::new(
            MemoryCvars::new(CommandDialect::Q2Classic)
                .with_flags("noset", "1", Q2_CVAR_FLAG_NOSET)
                .with("free", "1"),
        );
        assert!(!(bind_cvar_setting(&q2, spec("noset", CvarSettingKind::Toggle), None)
            .unwrap()
            .enabled)());
        assert!((bind_cvar_setting(&q2, spec("free", CvarSettingKind::Toggle), None)
            .unwrap()
            .enabled)());

        let q1: Rc<dyn SettingCvars> =
            Rc::new(MemoryCvars::new(CommandDialect::Q1Netquake).with_flags("any", "1", 0xFFFF));
        assert!((bind_cvar_setting(&q1, spec("any", CvarSettingKind::Toggle), None)
            .unwrap()
            .enabled)());
    }

    #[test]
    fn cvar_read_prefers_latched_value() {
        let cvars: Rc<dyn SettingCvars> =
            Rc::new(MemoryCvars::new(CommandDialect::Q3).with_latched("vid_mode", "3", "5"));
        let binding = bind_cvar_setting(
            &cvars,
            spec(
                "vid_mode",
                CvarSettingKind::Slider {
                    minimum: 0.0,
                    maximum: 8.0,
                    step: 1.0,
                },
            ),
            None,
        )
        .unwrap();
        match &binding.kind {
            SettingBindingKind::Slider { read, .. } => assert_eq!(read(), 5.0),
            _ => panic!("expected slider"),
        }
    }

    #[test]
    #[should_panic(expected = "Unknown cvar setting choice")]
    fn cvar_choice_rejects_unknown_value() {
        let cvars: Rc<dyn SettingCvars> = Rc::new(MemoryCvars::new(CommandDialect::Q3).with("snd_backend", "pulse"));
        let binding = bind_cvar_setting(
            &cvars,
            spec(
                "snd_backend",
                CvarSettingKind::Choice {
                    choices: vec![UiChoice {
                        id: "pulse".to_string(),
                        label: "Pulse".to_string(),
                    }],
                },
            ),
            None,
        )
        .unwrap();
        match &binding.kind {
            SettingBindingKind::Choice { write, .. } => write("nope"),
            _ => panic!("expected choice"),
        }
    }

    #[test]
    fn cvar_submit_only_keeps_draft_until_submit() {
        let cvars: Rc<dyn SettingCvars> = Rc::new(MemoryCvars::new(CommandDialect::Q3).with("player_name", "hero"));
        let binding = bind_cvar_setting(
            &cvars,
            spec(
                "player_name",
                CvarSettingKind::TextEntry {
                    maximum_length: 16,
                    submit_only: true,
                },
            ),
            None,
        )
        .unwrap();
        match &binding.kind {
            SettingBindingKind::TextEntry {
                read,
                write,
                commit: Some(commit),
                ..
            } => {
                write("draft");
                assert_eq!(read(), "draft");
                assert_eq!(cvars.find("player_name").unwrap().value, "hero");
                (commit.cancel)();
                assert_eq!(read(), "hero");
                write("draft-two");
                (commit.submit)("draft-two");
                assert_eq!(read(), "draft-two");
                assert_eq!(cvars.find("player_name").unwrap().value, "draft-two");
            }
            _ => panic!("expected submit-only text entry"),
        }
    }

    #[test]
    fn setting_control_maps_every_kind() {
        let registry = owner();
        let seat = registry.seat(0);
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        let seen = Rc::new(Cell::new(false));
        let seen_write = Rc::clone(&seen);
        let toggle = SettingBinding {
            id: UiControlId::new("ui:test:toggle").unwrap(),
            label: "Toggle".to_string(),
            category: SettingCategory::Input,
            enabled: Rc::new(|| true),
            kind: SettingBindingKind::Toggle {
                read: Rc::new(|| true),
                write: Rc::new(move |value| seen_write.set(value)),
            },
        };
        let control = setting_control(&toggle, &rect, seat.clone());
        match &control.kind {
            UiControlKind::Toggle { checked, on_change } => {
                assert!(*checked);
                on_change(seat.clone(), false);
            }
            _ => panic!("expected toggle control"),
        }
        assert!(!seen.get());

        let slider = SettingBinding {
            id: UiControlId::new("ui:test:slider").unwrap(),
            label: "Slider".to_string(),
            category: SettingCategory::Input,
            enabled: Rc::new(|| false),
            kind: SettingBindingKind::Slider {
                read: Rc::new(|| 50.0),
                write: Rc::new(|_| {}),
                minimum: 0.0,
                maximum: 100.0,
                step: 1.0,
                format_value: Some(Rc::new(format_percent)),
            },
        };
        let control = setting_control(&slider, &rect, seat.clone());
        assert!(!control.enabled);
        match &control.kind {
            UiControlKind::Slider { value, value_label, .. } => {
                assert_eq!(*value, 50.0);
                assert_eq!(value_label.as_deref(), Some("50%"));
            }
            _ => panic!("expected slider control"),
        }

        let submitted = Rc::new(RefCell::new(String::new()));
        let submitted_write = Rc::clone(&submitted);
        let direct = SettingBinding {
            id: UiControlId::new("ui:test:direct").unwrap(),
            label: "Direct".to_string(),
            category: SettingCategory::Input,
            enabled: Rc::new(|| true),
            kind: SettingBindingKind::TextEntry {
                read: Rc::new(|| "a".to_string()),
                write: Rc::new(move |value| *submitted_write.borrow_mut() = value.to_string()),
                maximum_length: 8,
                commit: None,
            },
        };
        let control = setting_control(&direct, &rect, seat.clone());
        match &control.kind {
            UiControlKind::TextEntry {
                on_submit,
                maximum_length,
                ..
            } => {
                assert_eq!(*maximum_length, 8);
                on_submit(seat.clone(), "b");
            }
            _ => panic!("expected text control"),
        }
        assert_eq!(*submitted.borrow(), "b");

        let committed = Rc::new(RefCell::new(String::new()));
        let committed_submit = Rc::clone(&committed);
        let draft = SettingBinding {
            id: UiControlId::new("ui:test:draft").unwrap(),
            label: "Draft".to_string(),
            category: SettingCategory::Input,
            enabled: Rc::new(|| true),
            kind: SettingBindingKind::TextEntry {
                read: Rc::new(|| "a".to_string()),
                write: Rc::new(|_| {}),
                maximum_length: 8,
                commit: Some(SettingCommit {
                    submit: Rc::new(move |value| {
                        *committed_submit.borrow_mut() = value.to_string();
                    }),
                    cancel: Rc::new(|| {}),
                }),
            },
        };
        let control = setting_control(&draft, &rect, seat.clone());
        match &control.kind {
            UiControlKind::TextEntry { on_submit, .. } => on_submit(seat.clone(), "c"),
            _ => panic!("expected text control"),
        }
        assert_eq!(*committed.borrow(), "c");

        let choice = SettingBinding {
            id: UiControlId::new("ui:test:choice").unwrap(),
            label: "Choice".to_string(),
            category: SettingCategory::Input,
            enabled: Rc::new(|| true),
            kind: SettingBindingKind::Choice {
                read: Rc::new(|| "b".to_string()),
                write: Rc::new(|_| {}),
                choices: Rc::new(|| {
                    vec![UiChoice {
                        id: "b".to_string(),
                        label: "B".to_string(),
                    }]
                }),
            },
        };
        let control = setting_control(&choice, &rect, seat.clone());
        match &control.kind {
            UiControlKind::Choice { selected, choices, .. } => {
                assert_eq!(selected.as_deref(), Some("b"));
                assert_eq!(choices.len(), 1);
            }
            _ => panic!("expected choice control"),
        }

        let activated = Rc::new(Cell::new(false));
        let activated_write = Rc::clone(&activated);
        let button = SettingBinding {
            id: UiControlId::new("ui:test:button").unwrap(),
            label: "Button".to_string(),
            category: SettingCategory::Input,
            enabled: Rc::new(|| true),
            kind: SettingBindingKind::Button {
                activate: Rc::new(move || activated_write.set(true)),
            },
        };
        let control = setting_control(&button, &rect, seat.clone());
        match &control.kind {
            UiControlKind::Button { on_activate } => on_activate(seat),
            _ => panic!("expected button control"),
        }
        assert!(activated.get());
    }

    #[test]
    #[should_panic(expected = "another seat")]
    fn setting_control_rejects_foreign_seat() {
        let registry = owner();
        let seat = registry.seat(0);
        let other = registry.seat(1);
        let rect = Rect {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        };
        let toggle = SettingBinding {
            id: UiControlId::new("ui:test:toggle").unwrap(),
            label: "Toggle".to_string(),
            category: SettingCategory::Input,
            enabled: Rc::new(|| true),
            kind: SettingBindingKind::Toggle {
                read: Rc::new(|| true),
                write: Rc::new(|_| {}),
            },
        };
        let control = setting_control(&toggle, &rect, seat);
        match &control.kind {
            UiControlKind::Toggle { on_change, .. } => on_change(other, false),
            _ => panic!("expected toggle control"),
        }
    }

    fn draw_texts(controller: &Rc<RefCell<NativeUiController>>, seat: SeatId) -> String {
        let registry = owner();
        let context = UiDrawContext {
            binding: SeatPresentationBinding {
                seat,
                client: registry.client(0, 0),
                viewport: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 640.0,
                    height: 480.0,
                },
                safe_area: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 640.0,
                    height: 480.0,
                },
                hud_scale: 1.0,
                presentation: PresentationSelection {
                    doppler: DopplerSelection::Source,
                    environment: EnvironmentSelection::Disabled,
                    assets: ContentId::new("test"),
                    hud: ProviderRef {
                        provider: "test".to_string(),
                        content: ContentId::new("hud"),
                    },
                    effects: ProviderRef {
                        provider: "test".to_string(),
                        content: ContentId::new("fx"),
                    },
                    audio: ProviderRef {
                        provider: "test".to_string(),
                        content: ContentId::new("audio"),
                    },
                },
            },
            time_ms: 0,
        };
        let commands = controller.borrow_mut().draw(&context).unwrap();
        let mut texts = Vec::new();
        for command in commands {
            if let UiDrawCommand::Text { text, .. } = command {
                texts.push(text);
            }
        }
        texts.join("\n")
    }

    #[test]
    fn settings_menus_register_draw_and_dispose() {
        let registry = owner();
        let seat = registry.seat(0);
        let controller = Rc::new(RefCell::new(NativeUiController::new(headless_options(seat.clone()))));
        let cvars: Rc<dyn SettingCvars> = Rc::new(MemoryCvars::new(CommandDialect::Q3).with("snd_volume", "0.5"));
        let audio = bind_cvar_setting(
            &cvars,
            spec(
                "snd_volume",
                CvarSettingKind::Slider {
                    minimum: 0.0,
                    maximum: 1.0,
                    step: 0.05,
                },
            ),
            None,
        )
        .unwrap();
        let reset_seen = Rc::new(Cell::new(false));
        let reset_write = Rc::clone(&reset_seen);
        let llm_service: Rc<RefCell<dyn LlmSettingsUi>> = Rc::new(RefCell::new(FakeLlm));
        let menus = register_settings_menus(
            &controller,
            &[audio],
            Some(Rc::clone(&llm_service)),
            Some(SettingReset {
                label: "Restore defaults".to_string(),
                apply: Rc::new(move || reset_write.set(true)),
            }),
        );
        assert_eq!(menus.root.as_str(), "menu:settings:root");
        assert!(controller.borrow().is_registered(&menus.root));
        assert!(controller
            .borrow()
            .is_registered(&UiMenuId::new("menu:settings:llm").unwrap()));
        let category = UiMenuId::new("menu:settings:audio:0").unwrap();
        assert!(controller.borrow().is_registered(&category));
        assert!(!controller
            .borrow()
            .is_registered(&UiMenuId::new("menu:settings:video:0").unwrap()));

        controller.borrow_mut().open_menu(&menus.root).unwrap();
        match controller.borrow_mut().state().focus {
            SeatInputFocus::Menu { menu, .. } => assert_eq!(menu, menus.root),
            _ => panic!("expected root menu focus"),
        }
        let root_text = draw_texts(&controller, seat.clone());
        assert!(root_text.contains("Audio"), "root draws category rows: {root_text}");
        assert!(root_text.contains("LLM options"), "root draws llm row: {root_text}");
        assert!(
            root_text.contains("Restore defaults"),
            "root draws reset row: {root_text}"
        );
        controller.borrow_mut().close_menu();
        controller.borrow_mut().open_menu(&category).unwrap();
        let category_text = draw_texts(&controller, seat);
        assert!(
            category_text.contains("snd_volume"),
            "category draws binding rows: {category_text}"
        );
        controller.borrow_mut().close_all();

        menus.dispose();
        assert!(!controller
            .borrow()
            .is_registered(&UiMenuId::new("menu:settings:root").unwrap()));
        assert!(!controller.borrow().is_registered(&category));
        assert!(!controller
            .borrow()
            .is_registered(&UiMenuId::new("menu:settings:llm").unwrap()));
        assert!(!reset_seen.get());
    }

    #[test]
    fn settings_menus_new_tracks_ids() {
        let registry = owner();
        let controller = Rc::new(RefCell::new(NativeUiController::new(headless_options(
            registry.seat(0),
        ))));
        let id = UiMenuId::new("menu:settings:audio:0").unwrap();
        controller.borrow_mut().register(
            id.clone(),
            Rc::new(move || UiMenu {
                scroll: None,
                id: UiMenuId::new("menu:settings:audio:0").unwrap(),
                title: "Audio".to_string(),
                full_screen: false,
                controls: Vec::new(),
                on_open: Rc::new(|_| {}),
                on_close: Rc::new(|_| {}),
            }),
        );
        let menus = SettingsMenus::new(
            &controller,
            UiMenuId::new("menu:settings:root").unwrap(),
            vec![id.clone()],
        );
        menus.dispose();
        assert!(!controller.borrow().is_registered(&id));
    }

    #[test]
    fn preferences_round_trip_through_cvars_and_local() {
        let registry = owner();
        let seat = registry.seat(0);
        let backing = Rc::new(MemoryCvars::new(CommandDialect::Q3));
        let cvars: Rc<dyn SettingCvars> = backing.clone();
        let prefs = Rc::new(SeatUiPreferences::new(seat.clone(), Some(cvars)));
        assert_eq!(prefs.seat(), seat);
        assert_eq!(prefs.values(), DEFAULT_UI_PREFERENCES);
        let mut custom = DEFAULT_UI_PREFERENCES;
        custom.hud_scale = 1.25;
        custom.typeface = Typeface::Bold;
        custom.color_mode = InterfaceColorMode::Monochrome;
        prefs.set_values(&custom);
        assert_eq!(prefs.values(), custom);
        assert_eq!(backing.get("ui_seat1_hudScale").as_deref(), Some("1.25"));

        let bindings = prefs.bindings();
        assert_eq!(bindings.len(), 11);
        let crosshair = bindings
            .iter()
            .find(|binding| binding.id.as_str() == "ui:accessibility:crosshair")
            .expect("crosshair row");
        write_toggle(crosshair, false);
        assert!(!prefs.values().crosshair);
        let reset = bindings
            .iter()
            .find(|binding| binding.id.as_str() == "ui:accessibility:reset")
            .expect("reset row");
        match &reset.kind {
            SettingBindingKind::Button { activate } => activate(),
            _ => panic!("expected reset button"),
        }
        assert_eq!(prefs.values(), DEFAULT_UI_PREFERENCES);

        let local = Rc::new(SeatUiPreferences::new(seat, None));
        local.set_values(&custom);
        assert_eq!(local.values(), custom);
    }

    #[test]
    fn gamepad_validation_matches_donor_ranges() {
        assert!(validate_gamepad_tuning(&DEFAULT_GAMEPAD_TUNING).is_ok());
        let mut bad = DEFAULT_GAMEPAD_TUNING;
        bad.move_stick = StickCurve::Radial {
            deadzone: 0.9,
            outer_threshold: 0.2,
            exponent: 2.0,
        };
        assert!(validate_gamepad_tuning(&bad).is_err());
        let mut bad = DEFAULT_GAMEPAD_TUNING;
        bad.look_stick = StickCurve::Axial {
            deadzone: 0.1,
            exponent: 0.0,
        };
        assert!(validate_gamepad_tuning(&bad).is_err());
        let mut bad = DEFAULT_GAMEPAD_TUNING;
        bad.trigger_threshold = 2.0;
        assert!(validate_gamepad_tuning(&bad).is_err());
        let mut bad = DEFAULT_GAMEPAD_TUNING;
        bad.yaw_degrees_per_second = f32::NAN;
        assert!(validate_gamepad_tuning(&bad).is_err());
    }

    #[test]
    fn audio_format_validation_and_rates() {
        assert_eq!(audio_output_rates(), vec![11025, 22050, 44100, 48000]);
        assert!(audio_output_format_validate(&DEFAULT_AUDIO_OUTPUT_FORMAT).is_ok());
        for candidate in [
            AudioOutputFormat {
                sample_rate: 7999,
                sample_bits: 16,
                channels: 2,
            },
            AudioOutputFormat {
                sample_rate: 44100,
                sample_bits: 24,
                channels: 2,
            },
            AudioOutputFormat {
                sample_rate: 44100,
                sample_bits: 16,
                channels: 3,
            },
        ] {
            assert!(audio_output_format_validate(&candidate).is_err());
        }
    }

    #[test]
    fn audio_settings_bind_device_format_and_volumes() {
        let service: Rc<dyn SettingsValueService<AudioSettings>> = Rc::new(MemoryService::new(AudioSettings {
            effects_volume: 0.8,
            music_volume: 0.6,
        }));
        let format = Rc::new(RefCell::new(DEFAULT_AUDIO_OUTPUT_FORMAT));
        let output = Rc::new(MemoryOutput {
            selected: RefCell::new(None),
            devices: vec!["hdmi".to_string()],
            reports: RefCell::new(Vec::new()),
            format: Some(Rc::new(MemoryFormat {
                format: Rc::clone(&format),
            })),
        });
        let bindings = bind_audio_settings(service, Some(output.clone() as Rc<dyn AudioOutputSettings>));
        assert_eq!(bindings.len(), 6);
        let device = bindings
            .iter()
            .find(|binding| binding.id.as_str() == "ui:audio:device")
            .expect("device row");
        match &device.kind {
            SettingBindingKind::Choice { read, write, choices } => {
                assert_eq!(read(), "default");
                let ids: Vec<String> = choices().into_iter().map(|choice| choice.id).collect();
                assert!(ids.contains(&"default".to_string()));
                assert!(ids.contains(&"device:hdmi".to_string()));
                write("device:hdmi");
                assert_eq!(read(), "device:hdmi");
                write("bogus");
                assert_eq!(output.reports.borrow().len(), 1);
            }
            _ => panic!("expected device choice"),
        }
        let rate = bindings
            .iter()
            .find(|binding| binding.id.as_str() == "ui:audio:rate")
            .expect("rate row");
        match &rate.kind {
            SettingBindingKind::Choice { read, write, choices } => {
                assert_eq!(read(), "44100");
                assert!(choices().iter().any(|choice| choice.label == "48000 Hz"));
                write("48000");
                assert_eq!(format.borrow().sample_rate, 48000);
            }
            _ => panic!("expected rate choice"),
        }
    }

    #[test]
    fn music_playlist_and_geometry_tolerate_missing_registry() {
        assert!(bind_music_playlist_settings(None, None).is_empty());
        assert!(bind_audio_geometry_settings(None).is_empty());
        let cvars: Rc<dyn SettingCvars> = Rc::new(
            MemoryCvars::new(CommandDialect::Q3)
                .with("music_shuffle", "0")
                .with("music_menu_track", "track2")
                .with("s_geometryAcoustics", "1"),
        );
        let tracks: Rc<dyn Fn() -> Vec<String>> = Rc::new(|| vec!["track1".to_string(), "track2".to_string()]);
        let music = bind_music_playlist_settings(Some(&cvars), Some(tracks));
        assert_eq!(music.len(), 2);
        assert!(!read_toggle(&music[0]));
        write_toggle(&music[0], true);
        assert_eq!(cvars.find("music_shuffle").unwrap().value, "1");
        match &music[1].kind {
            SettingBindingKind::Choice { read, choices, .. } => {
                assert_eq!(read(), "track2");
                let ids: Vec<String> = choices().into_iter().map(|choice| choice.id).collect();
                assert!(ids.contains(&"auto".to_string()));
                assert!(ids.contains(&"track1".to_string()));
            }
            _ => panic!("expected menu-track choice"),
        }
        let geometry = bind_audio_geometry_settings(Some(&cvars));
        assert_eq!(geometry.len(), 1);
        assert!(read_toggle(&geometry[0]));
    }

    #[test]
    fn input_settings_sample_host_and_gate_look_spring() {
        let host: Rc<RefCell<dyn InputTuningHost>> = Rc::new(RefCell::new(MemoryHost::new(CommandDialect::Q1Netquake)));
        let vibration: VibrationService = Rc::new(MemoryService::new(ControllerVibrationSettings {
            controller_vibration: true,
            controller_vibration_strength: 0.5,
        }));
        let bindings = bind_input_settings(&host, Some(vibration));
        assert!(bindings.len() > 20);
        let sensitivity = bindings
            .iter()
            .find(|binding| binding.id.as_str() == "ui:input:sensitivity")
            .expect("sensitivity row");
        match &sensitivity.kind {
            SettingBindingKind::Slider { read, write, .. } => {
                assert_eq!(read(), 3.0);
                write(4.0);
            }
            _ => panic!("expected sensitivity slider"),
        }
        assert_eq!(host.borrow().mouse_tuning().sensitivity, 4.0);

        let yaw = bindings
            .iter()
            .find(|binding| binding.id.as_str() == "ui:input:mouse-yaw")
            .expect("yaw row");
        match &yaw.kind {
            SettingBindingKind::Slider {
                read,
                write,
                format_value,
                ..
            } => {
                assert!((read() - 100.0).abs() < 0.01);
                assert_eq!(format_value.as_ref().unwrap()(read()), "100%");
                write(50.0);
            }
            _ => panic!("expected yaw slider"),
        }
        assert!((host.borrow().mouse_tuning().yaw - 0.011).abs() < 0.0001);

        host.borrow().set_mouse_tuning(&MouseTuningView {
            free_look: false,
            ..host.borrow().mouse_tuning()
        });
        let spring = bindings
            .iter()
            .find(|binding| binding.id.as_str() == "ui:input:lookspring")
            .expect("look-spring row");
        assert!((spring.enabled)());

        let q3: Rc<RefCell<dyn InputTuningHost>> = Rc::new(RefCell::new(MemoryHost::new(CommandDialect::Q3)));
        let q3_bindings = bind_input_settings(&q3, None);
        assert!(!q3_bindings
            .iter()
            .any(|binding| binding.id.as_str() == "ui:input:controller-vibration"));
        let q3_spring = q3_bindings
            .iter()
            .find(|binding| binding.id.as_str() == "ui:input:lookspring")
            .expect("look-spring row");
        assert!(!(q3_spring.enabled)());
    }

    #[test]
    fn gamepad_settings_cover_curves_and_previews() {
        let host: Rc<RefCell<dyn InputTuningHost>> = Rc::new(RefCell::new(MemoryHost::new(CommandDialect::Q3)));
        let bindings = bind_gamepad_settings(&host);
        let curve = bindings
            .iter()
            .find(|binding| binding.id.as_str() == "ui:input:look-curve-type")
            .expect("curve-type row");
        match &curve.kind {
            SettingBindingKind::Choice { read, write, .. } => {
                assert_eq!(read(), "radial");
                write("axial");
                assert_eq!(read(), "axial");
            }
            _ => panic!("expected curve-type choice"),
        }
        let outer = bindings
            .iter()
            .find(|binding| binding.id.as_str() == "ui:input:look-outer")
            .expect("outer row");
        assert!(!(outer.enabled)());
        let preview = bindings
            .iter()
            .find(|binding| binding.id.as_str() == "ui:input:look-preview-x")
            .expect("preview row");
        assert!(!(preview.enabled)());
        match &preview.kind {
            SettingBindingKind::Slider { read, format_value, .. } => {
                assert_eq!(read(), 0.0);
                assert_eq!(format_value.as_ref().unwrap()(0.25), "Raw 0.00 / 0.25");
            }
            _ => panic!("expected preview slider"),
        }
        let deadzone = bindings
            .iter()
            .find(|binding| binding.id.as_str() == "ui:input:move-deadzone")
            .expect("deadzone row");
        match &deadzone.kind {
            SettingBindingKind::Slider { write, .. } => write(0.3),
            _ => panic!("expected deadzone slider"),
        }
        assert!((host.borrow().gamepad_tuning().move_stick.deadzone() - 0.3).abs() < 0.0001);
    }

    #[test]
    #[should_panic(expected = "Unknown controller curve")]
    fn gamepad_curve_rejects_unknown_kind() {
        let host: Rc<RefCell<dyn InputTuningHost>> = Rc::new(RefCell::new(MemoryHost::new(CommandDialect::Q3)));
        let bindings = bind_gamepad_settings(&host);
        let curve = bindings
            .iter()
            .find(|binding| binding.id.as_str() == "ui:input:move-curve-type")
            .expect("curve-type row");
        match &curve.kind {
            SettingBindingKind::Choice { write, .. } => write("spline"),
            _ => panic!("expected curve-type choice"),
        }
    }
}
