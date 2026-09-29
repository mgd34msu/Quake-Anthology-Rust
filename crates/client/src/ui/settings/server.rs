//! Host server settings menu: paged rows, per-setting detail, and profiles.
//!
//! Donor provenance: `src/ui/settings/server.ts` in full. View shapes mirror
//! `src/settings/server/types.ts` (`BoundServerSetting`, `ServerSettingId`,
//! `ServerSettingStatus`) and the profile helpers mirror
//! `src/settings/server/profile.ts` (`captureServerProfile`,
//! `applyServerProfile`); parsing, scheduling, and storage stay with the
//! existing source owner. The profile payload below is a stable UI-local line
//! encoding, not the donor JSON, because this crate has no JSON dependency.
//! The menu links to the rotation submenu from [`super::rotation`].

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use qa_core::identity::SeatId;

use crate::ui::common::controller::NativeUiController;
use crate::ui::common::layout::{menu_row, MenuRowOptions};
use crate::ui::types::{SeatUiController as _, UiControl, UiControlId, UiControlKind, UiMenu, UiMenuId};

use super::rotation::{
    register_rotation_menu_tracked, ServerApplyAt, ServerBindingView, ServerRotationBindings, ServerSettingKindView,
    MAP_ROTATION_SETTING_ID,
};
use super::{setting_control, SettingBinding, SettingBindingKind, SettingCategory, SettingsMenus};

/// Server settings root id (donor `menu:server:settings`).
const ROOT_MENU_ID: &str = "menu:server:settings";
/// Server setting detail id (donor `menu:server:detail`).
const DETAIL_MENU_ID: &str = "menu:server:detail";
/// Server profiles id (donor `menu:server:profiles`).
const PROFILES_MENU_ID: &str = "menu:server:profiles";
/// Settings shown per root page (donor page size 7).
const PAGE_SIZE: usize = 7;
/// Draft width for non-text controls (donor `else` branch length 24).
const DRAFT_MAXIMUM_LENGTH: usize = 24;
/// Profile-name rejection message (donor `profile`).
const PROFILE_NAME_ERROR: &str = "Use 1–32 letters, digits, hyphens or underscores.";
/// Missing-selection message (donor `apply` / `binding`).
const MISSING_SETTING_ERROR: &str = "This setting is no longer available.";
/// Profile payload header: magic plus version.
const PROFILE_HEADER: &str = "server-profile 1";

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

/// Synchronous profile storage behind the profiles menu.
///
/// The donor `ConfigStore` is async; this UI-local surface is synchronous so
/// menu callbacks never block on the draw path. Callers with slow backends
/// must run them off the draw path. Paths follow the donor
/// [`server_profile_path`] shape; payloads use the
/// [`encode_server_profile`] line encoding.
pub trait ServerProfileStore {
    /// Read one stored payload, or `None` when the profile does not exist.
    fn read(&self, name: &str) -> Option<String>;
    /// Write one stored payload, reporting the backend message on failure.
    fn write(&mut self, name: &str, value: &str) -> Result<(), String>;
}

/// Decoded server profile: `(setting id, desired value)` pairs in file order.
pub type ProfileData = Vec<(String, String)>;

/// Profile path for one validated name (donor `` `servers/${name}.json` ``).
///
/// The path keeps the donor shape; the payload is the UI-local
/// [`encode_server_profile`] line encoding, not JSON.
#[must_use]
pub fn server_profile_path(name: &str) -> String {
    format!("servers/{name}.json")
}

/// Whether a profile name matches the donor
/// `^[A-Za-z0-9][A-Za-z0-9_-]{0,31}$`: 1-32 characters, leading letter or
/// digit, then letters, digits, hyphens, or underscores.
#[must_use]
pub fn is_valid_profile_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    match bytes.next() {
        Some(first) if first.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    name.len() <= 32 && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// Escape one profile field: backslash, line breaks, and the `=` separator.
fn escape_profile_field(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for char in text.chars() {
        match char {
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '=' => escaped.push_str("\\="),
            _ => escaped.push(char),
        }
    }
    escaped
}

/// Unescape one profile field, rejecting dangling or unknown escapes.
fn unescape_profile_field(text: &str) -> Result<String, String> {
    let mut unescaped = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(char) = chars.next() {
        if char != '\\' {
            unescaped.push(char);
            continue;
        }
        match chars.next() {
            Some('\\') => unescaped.push('\\'),
            Some('n') => unescaped.push('\n'),
            Some('r') => unescaped.push('\r'),
            Some('=') => unescaped.push('='),
            _ => return Err("Unsupported server profile".to_string()),
        }
    }
    Ok(unescaped)
}

/// Split one payload line at the first unescaped `=`.
fn split_profile_line(line: &str) -> Result<(&str, &str), String> {
    let mut escaped = false;
    for (index, char) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match char {
            '\\' => escaped = true,
            '=' => return Ok((&line[..index], &line[index + 1..])),
            _ => {}
        }
    }
    Err("Unsupported server profile".to_string())
}

/// Encode one profile with the STABLE line format.
///
/// The payload is the header line `server-profile 1`, then one
/// `escaped-id=escaped-value` line per entry in order, every line terminated
/// by `\n`. Fields escape `\` as `\\`, line feed as `\n`, carriage return
/// as `\r`, and `=` as `\=`; the separator is the first unescaped `=`.
/// Encoding rejects empty ids and duplicate ids, mirroring the donor
/// `parseServerProfile` validation on the save path.
pub fn encode_server_profile(profile: &[(String, String)]) -> Result<String, String> {
    let mut seen = HashSet::new();
    let mut payload = String::from(PROFILE_HEADER);
    payload.push('\n');
    for (id, value) in profile {
        if id.is_empty() {
            return Err("Server profile setting has no id".to_string());
        }
        if !seen.insert(id) {
            return Err(format!("Duplicate server profile setting {id}"));
        }
        payload.push_str(&escape_profile_field(id));
        payload.push('=');
        payload.push_str(&escape_profile_field(value));
        payload.push('\n');
    }
    Ok(payload)
}

/// Decode one profile payload, accepting a missing final newline.
///
/// Decoding rejects unknown headers, malformed lines, bad escapes, empty
/// ids, and duplicate ids, mirroring the donor `parseServerProfile`
/// validation. Values are not validated against definitions here; the owner
/// validates each value when [`apply_server_profile`] writes it.
pub fn decode_server_profile(payload: &str) -> Result<ProfileData, String> {
    let body = payload.strip_suffix('\n').unwrap_or(payload);
    let mut lines = body.split('\n');
    match lines.next() {
        Some(PROFILE_HEADER) => {}
        _ => return Err("Unsupported server profile".to_string()),
    }
    let mut profile = ProfileData::new();
    let mut seen = HashSet::new();
    for line in lines {
        if line.is_empty() {
            return Err("Unsupported server profile".to_string());
        }
        let (raw_id, raw_value) = split_profile_line(line)?;
        let id = unescape_profile_field(raw_id)?;
        if id.is_empty() {
            return Err("Server profile setting has no id".to_string());
        }
        if !seen.insert(id.clone()) {
            return Err(format!("Duplicate server profile setting {id}"));
        }
        profile.push((id, unescape_profile_field(raw_value)?));
    }
    Ok(profile)
}

/// Capture desired values the way the donor `captureServerProfile` does.
#[must_use]
pub fn capture_server_profile(bindings: &[ServerBindingView]) -> ProfileData {
    bindings
        .iter()
        .map(|binding| (binding.definition.id.clone(), binding.desired.clone()))
        .collect()
}

/// Apply one profile through a per-setting writer the way the donor
/// `applyServerProfile` does: reject duplicate ids before any write, then
/// write entries in order, stopping at the first owner rejection.
///
/// Unlike the donor, values cannot be pre-validated UI-locally, so a late
/// rejection may leave earlier entries applied; owners that need atomicity
/// must stage the writes themselves.
pub fn apply_server_profile(
    profile: &[(String, String)],
    write: &dyn Fn(&str, &str) -> Result<String, String>,
) -> Result<(), String> {
    let mut seen = HashSet::new();
    for (id, _) in profile {
        if !seen.insert(id) {
            return Err(format!("Duplicate server profile setting {id}"));
        }
    }
    for (id, value) in profile {
        write(id, value).map(|_| ())?;
    }
    Ok(())
}

/// Write one setting value, returning the normalized desired value or the
/// owner rejection message.
pub type ServerWriteFn = Rc<dyn Fn(&str, &str) -> Result<String, String>>;

/// Live host surface behind the server menus.
#[derive(Clone)]
pub struct HostServerSettingsUi {
    /// Read every binding snapshot.
    pub bindings: Rc<dyn Fn() -> Vec<ServerBindingView>>,
    /// Write one setting value.
    pub write_setting: ServerWriteFn,
    /// Profile storage.
    pub store: Rc<RefCell<dyn ServerProfileStore>>,
}

/// Server menus draft state (donor `page`, `selected`, `draft`, `error`,
/// `profileName`, `profileMessage`, `busy`).
#[derive(Debug, Clone)]
pub struct ServerSettingsMenuState {
    /// Root page.
    pub page: usize,
    /// Selected setting id.
    pub selected: Option<String>,
    /// Detail draft value.
    pub draft: String,
    /// Latched detail error.
    pub error: String,
    /// Profile name draft.
    pub profile_name: String,
    /// Latched profile message.
    pub profile_message: String,
    /// Profile operation in flight (reentrancy guard; operations are
    /// synchronous so this is never observed set).
    pub busy: bool,
}

impl Default for ServerSettingsMenuState {
    /// Donor initial state, including the `"default"` profile name.
    fn default() -> Self {
        Self {
            page: 0,
            selected: None,
            draft: String::new(),
            error: String::new(),
            profile_name: "default".to_string(),
            profile_message: String::new(),
            busy: false,
        }
    }
}

/// Registered server menu ids.
#[derive(Debug, Clone)]
pub struct ServerMenuIds {
    /// Settings root.
    pub root: UiMenuId,
    /// Setting detail.
    pub detail: UiMenuId,
    /// Profiles.
    pub profiles: UiMenuId,
    /// Linked rotation submenu root.
    pub rotation: UiMenuId,
}

/// Inputs rebuilding the server menus.
#[derive(Clone)]
pub struct ServerMenuInputs {
    /// Live host surface.
    pub host: HostServerSettingsUi,
    /// Controller owning the menu stack.
    pub controller: Rc<RefCell<NativeUiController>>,
    /// Seat owning detail controls.
    pub seat: SeatId,
    /// Draft state.
    pub state: Rc<RefCell<ServerSettingsMenuState>>,
}

/// Timing line for one binding (donor `timing`).
#[must_use]
pub fn server_setting_timing(binding: &ServerBindingView) -> String {
    let when = match binding.apply_at {
        ServerApplyAt::Live => "next source update",
        ServerApplyAt::NextMatch => "next match",
        ServerApplyAt::NextMap => "next map",
        ServerApplyAt::Restart => "server restart",
    };
    if binding.pending {
        format!("Pending: applies on {when}")
    } else if binding.apply_at == ServerApplyAt::Live {
        "Applies during play".to_string()
    } else {
        format!("Changes apply on {when}")
    }
}

/// Wrap text into 60-column lines the way the donor `lines` does, including
/// its quirk: a word longer than the width pushes the empty remainder first.
#[must_use]
pub fn wrap_lines(text: &str) -> Vec<String> {
    let mut wrapped = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if line.chars().count() + word.chars().count() > 60 {
            wrapped.push(std::mem::take(&mut line));
            line.push_str(word);
        } else {
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
    }
    if !line.is_empty() {
        wrapped.push(line);
    }
    wrapped
}

/// Find the selected binding snapshot.
fn selected_binding(host: &HostServerSettingsUi, selected: &Option<String>) -> Option<ServerBindingView> {
    let selected = selected.as_ref()?;
    (host.bindings)()
        .into_iter()
        .find(|binding| &binding.definition.id == selected)
}

/// Apply the detail draft (donor `apply`).
fn apply_server_draft(state: &Rc<RefCell<ServerSettingsMenuState>>, host: &HostServerSettingsUi) {
    let selected = state.borrow().selected.clone();
    let Some(current) = selected_binding(host, &selected) else {
        state.borrow_mut().error = MISSING_SETTING_ERROR.to_string();
        return;
    };
    let draft = state.borrow().draft.clone();
    match (host.write_setting)(&current.definition.id, &draft) {
        Ok(normalized) => {
            let mut state = state.borrow_mut();
            state.draft = normalized;
            state.error.clear();
        }
        Err(message) => state.borrow_mut().error = message,
    }
}

/// Profile operation (donor `profile` action argument).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProfileAction {
    /// Save current desired settings.
    Save,
    /// Load and apply a profile.
    Load,
}

/// Run one synchronous profile operation (donor `profile`).
fn run_profile_action(
    state: &Rc<RefCell<ServerSettingsMenuState>>,
    host: &HostServerSettingsUi,
    action: ProfileAction,
) {
    if state.borrow().busy {
        return;
    }
    if !is_valid_profile_name(&state.borrow().profile_name) {
        state.borrow_mut().profile_message = PROFILE_NAME_ERROR.to_string();
        return;
    }
    state.borrow_mut().busy = true;
    state.borrow_mut().profile_message.clear();
    let name = state.borrow().profile_name.clone();
    let path = server_profile_path(&name);
    let outcome = match action {
        ProfileAction::Save => {
            let data = capture_server_profile(&(host.bindings)());
            match encode_server_profile(&data) {
                Ok(payload) => host.store.borrow_mut().write(&path, &payload),
                Err(message) => Err(message),
            }
        }
        ProfileAction::Load => {
            let stored = host.store.borrow().read(&path);
            match stored {
                None => Err(format!("Profile {name} does not exist.")),
                Some(payload) => {
                    decode_server_profile(&payload).and_then(|data| apply_server_profile(&data, &*host.write_setting))
                }
            }
        }
    };
    let mut state = state.borrow_mut();
    state.profile_message = match outcome {
        Ok(()) => format!(
            "{} {name}.",
            if action == ProfileAction::Save {
                "Saved"
            } else {
                "Loaded"
            }
        ),
        Err(message) => message,
    };
    state.busy = false;
}

/// Build one button row (donor `button`).
fn server_button(id: &str, label: String, row: i32, enabled: bool, on_activate: Rc<dyn Fn()>) -> UiControl {
    UiControl {
        id: control_id(id),
        label,
        rect: menu_row(row, &MenuRowOptions::default()),
        enabled,
        visible: true,
        kind: UiControlKind::Button {
            on_activate: Rc::new(move |_| on_activate()),
        },
    }
}

/// Build one disabled info row (donor `info`).
fn server_info(label: String, row: i32, suffix: &str) -> UiControl {
    UiControl {
        id: control_id(&format!("ui:server:info-{suffix}")),
        label,
        rect: menu_row(row, &MenuRowOptions::default()),
        enabled: false,
        visible: true,
        kind: UiControlKind::Button {
            on_activate: Rc::new(|_| {}),
        },
    }
}

/// Build the back row (donor `back`).
fn server_back(controller: &Rc<RefCell<NativeUiController>>) -> UiControl {
    let back_controller = Rc::clone(controller);
    server_button(
        "ui:server:back",
        "Back".to_string(),
        11,
        true,
        Rc::new(move || {
            back_controller.borrow_mut().close_menu();
        }),
    )
}

/// Build the paged settings root (donor root factory).
#[must_use]
pub fn build_server_root_menu(ids: &ServerMenuIds, inputs: &ServerMenuInputs) -> UiMenu {
    let available = (inputs.host.bindings)();
    let pages = available.len().div_ceil(PAGE_SIZE).max(1);
    let page = {
        let mut state = inputs.state.borrow_mut();
        state.page = state.page.min(pages - 1);
        state.page
    };
    let mut controls: Vec<UiControl> = available
        .iter()
        .skip(page * PAGE_SIZE)
        .take(PAGE_SIZE)
        .enumerate()
        .map(|(index, binding)| {
            let id = format!("ui:server:{}", binding.definition.id);
            let label = format!(
                "{}: {}{}",
                binding.definition.label,
                binding.desired,
                if binding.pending { " (pending)" } else { "" }
            );
            let row = binding.clone();
            let row_state = Rc::clone(&inputs.state);
            let open_controller = Rc::clone(&inputs.controller);
            let target = if row.definition.id == MAP_ROTATION_SETTING_ID {
                ids.rotation.clone()
            } else {
                ids.detail.clone()
            };
            server_button(
                &id,
                label,
                index as i32,
                true,
                Rc::new(move || {
                    let mut state = row_state.borrow_mut();
                    state.selected = Some(row.definition.id.clone());
                    state.draft = row.desired.clone();
                    state.error.clear();
                    drop(state);
                    let _ = open_controller.borrow_mut().open_menu(&target);
                }),
            )
        })
        .collect();
    let previous_state = Rc::clone(&inputs.state);
    let next_state = Rc::clone(&inputs.state);
    let profiles_controller = Rc::clone(&inputs.controller);
    let profiles_target = ids.profiles.clone();
    controls.push(server_button(
        "ui:server:previous",
        "Previous page".to_string(),
        7,
        page > 0,
        Rc::new(move || {
            let mut state = previous_state.borrow_mut();
            state.page = state.page.saturating_sub(1);
        }),
    ));
    controls.push(server_button(
        "ui:server:next",
        "Next page".to_string(),
        8,
        page + 1 < pages,
        Rc::new(move || {
            let mut state = next_state.borrow_mut();
            state.page = state.page.saturating_add(1);
        }),
    ));
    controls.push(server_button(
        "ui:server:profiles",
        "Save or load profile".to_string(),
        9,
        true,
        Rc::new(move || {
            let _ = profiles_controller.borrow_mut().open_menu(&profiles_target);
        }),
    ));
    controls.push(server_back(&inputs.controller));
    UiMenu {
        scroll: None,
        id: ids.root.clone(),
        title: format!("Server settings {}/{}", page + 1, pages),
        full_screen: false,
        controls,
        on_open: Rc::new(|_| {}),
        on_close: Rc::new(|_| {}),
    }
}

/// Build the detail value control for toggle and choice kinds through
/// [`setting_control`] (donor `settingControl` branch).
fn detail_setting_control(inputs: &ServerMenuInputs, binding: &ServerBindingView) -> Option<UiControl> {
    let rows = MenuRowOptions::default();
    match &binding.definition.kind {
        ServerSettingKindView::Toggle => {
            let read_state = Rc::clone(&inputs.state);
            let write_state = Rc::clone(&inputs.state);
            Some(setting_control(
                &SettingBinding {
                    id: control_id("ui:server:value"),
                    label: "Requested value".to_string(),
                    category: SettingCategory::Network,
                    enabled: Rc::new(|| true),
                    kind: SettingBindingKind::Toggle {
                        read: Rc::new(move || read_state.borrow().draft == "1"),
                        write: Rc::new(move |value| {
                            write_state.borrow_mut().draft = if value { "1".to_string() } else { "0".to_string() };
                        }),
                    },
                },
                &menu_row(0, &rows),
                inputs.seat.clone(),
            ))
        }
        ServerSettingKindView::Choice { choices } => {
            let read_state = Rc::clone(&inputs.state);
            let write_state = Rc::clone(&inputs.state);
            let listed = choices.clone();
            Some(setting_control(
                &SettingBinding {
                    id: control_id("ui:server:value"),
                    label: "Requested value".to_string(),
                    category: SettingCategory::Network,
                    enabled: Rc::new(|| true),
                    kind: SettingBindingKind::Choice {
                        read: Rc::new(move || read_state.borrow().draft.clone()),
                        write: Rc::new(move |value| {
                            write_state.borrow_mut().draft = value.to_string();
                        }),
                        choices: Rc::new(move || listed.clone()),
                    },
                },
                &menu_row(0, &rows),
                inputs.seat.clone(),
            ))
        }
        ServerSettingKindView::Slider { .. }
        | ServerSettingKindView::TextEntry { .. }
        | ServerSettingKindView::Other => None,
    }
}

/// Build the setting detail menu (donor detail factory).
#[must_use]
pub fn build_server_detail_menu(ids: &ServerMenuIds, inputs: &ServerMenuInputs) -> UiMenu {
    let rows = MenuRowOptions::default();
    let (selected, draft, error) = {
        let state = inputs.state.borrow();
        (state.selected.clone(), state.draft.clone(), state.error.clone())
    };
    let current = selected_binding(&inputs.host, &selected);
    let mut controls: Vec<UiControl> = Vec::new();
    if let Some(binding) = current.as_ref() {
        match detail_setting_control(inputs, binding) {
            Some(control) => controls.push(control),
            None => {
                let maximum_length = match binding.definition.kind {
                    ServerSettingKindView::TextEntry { maximum_length } => maximum_length,
                    _ => DRAFT_MAXIMUM_LENGTH,
                };
                let change_state = Rc::clone(&inputs.state);
                let submit_state = Rc::clone(&inputs.state);
                let submit_host = inputs.host.clone();
                controls.push(UiControl {
                    id: control_id("ui:server:value"),
                    label: "Requested value".to_string(),
                    rect: menu_row(0, &rows),
                    enabled: true,
                    visible: true,
                    kind: UiControlKind::TextEntry {
                        masked: false,
                        text: draft,
                        maximum_length,
                        on_change: Rc::new(move |_, value| {
                            change_state.borrow_mut().draft = value.to_string();
                        }),
                        on_submit: Rc::new(move |_, _| {
                            apply_server_draft(&submit_state, &submit_host);
                        }),
                    },
                });
            }
        }
        controls.push(server_info(
            format!("Desired: {}   Effective: {}", binding.desired, binding.effective),
            1,
            "desired",
        ));
        controls.push(server_info(
            format!("Default: {}", binding.definition.default_value),
            2,
            "default",
        ));
        controls.push(server_info(server_setting_timing(binding), 3, "timing"));
        for (index, line) in wrap_lines(&binding.definition.description).iter().take(2).enumerate() {
            controls.push(server_info(
                line.clone(),
                4 + index as i32,
                &format!("description-{index}"),
            ));
        }
        if let ServerSettingKindView::Slider {
            minimum,
            maximum,
            integer,
        } = binding.definition.kind
        {
            controls.push(server_info(
                format!(
                    "{} from {minimum} to {maximum}",
                    if integer { "Whole numbers" } else { "Numbers" }
                ),
                6,
                "range",
            ));
        }
        let apply_state = Rc::clone(&inputs.state);
        let apply_host = inputs.host.clone();
        let default_state = Rc::clone(&inputs.state);
        let default_value = binding.definition.default_value.clone();
        controls.push(server_button(
            "ui:server:apply",
            "Apply".to_string(),
            7,
            true,
            Rc::new(move || apply_server_draft(&apply_state, &apply_host)),
        ));
        controls.push(server_button(
            "ui:server:default",
            "Use default".to_string(),
            8,
            true,
            Rc::new(move || {
                let mut state = default_state.borrow_mut();
                state.draft = default_value.clone();
                state.error.clear();
            }),
        ));
    } else {
        controls.push(server_info(MISSING_SETTING_ERROR.to_string(), 0, "missing"));
    }
    for (index, line) in wrap_lines(&error).iter().take(2).enumerate() {
        controls.push(server_info(line.clone(), 9 + index as i32, &format!("error-{index}")));
    }
    controls.push(server_back(&inputs.controller));
    UiMenu {
        scroll: None,
        id: ids.detail.clone(),
        title: current
            .map(|binding| binding.definition.label)
            .unwrap_or_else(|| "Server setting".to_string()),
        full_screen: false,
        controls,
        on_open: Rc::new(|_| {}),
        on_close: Rc::new(|_| {}),
    }
}

/// Build the profiles menu (donor profiles factory).
#[must_use]
pub fn build_server_profiles_menu(ids: &ServerMenuIds, inputs: &ServerMenuInputs) -> UiMenu {
    let rows = MenuRowOptions::default();
    let (profile_name, profile_message, busy) = {
        let state = inputs.state.borrow();
        (state.profile_name.clone(), state.profile_message.clone(), state.busy)
    };
    let change_state = Rc::clone(&inputs.state);
    let save_state = Rc::clone(&inputs.state);
    let save_host = inputs.host.clone();
    let load_state = Rc::clone(&inputs.state);
    let load_host = inputs.host.clone();
    let mut controls = vec![
        UiControl {
            id: control_id("ui:server:profile-name"),
            label: "Profile name".to_string(),
            rect: menu_row(0, &rows),
            enabled: !busy,
            visible: true,
            kind: UiControlKind::TextEntry {
                masked: false,
                text: profile_name,
                maximum_length: 32,
                on_change: Rc::new(move |_, value| {
                    change_state.borrow_mut().profile_name = value.to_string();
                }),
                on_submit: Rc::new(|_, _| {}),
            },
        },
        server_info(
            "A short name, such as league-night. No file paths.".to_string(),
            1,
            "profile-help",
        ),
        server_button(
            "ui:server:save",
            "Save current desired settings".to_string(),
            3,
            !busy,
            Rc::new(move || run_profile_action(&save_state, &save_host, ProfileAction::Save)),
        ),
        server_button(
            "ui:server:load",
            "Load and apply profile".to_string(),
            4,
            !busy,
            Rc::new(move || run_profile_action(&load_state, &load_host, ProfileAction::Load)),
        ),
        server_info(
            "Loaded settings keep their normal apply timing.".to_string(),
            5,
            "profile-timing",
        ),
    ];
    let message = if busy {
        "Working…".to_string()
    } else {
        profile_message
    };
    for (index, line) in wrap_lines(&message).iter().take(3).enumerate() {
        controls.push(server_info(
            line.clone(),
            7 + index as i32,
            &format!("profile-message-{index}"),
        ));
    }
    controls.push(server_back(&inputs.controller));
    UiMenu {
        scroll: None,
        id: ids.profiles.clone(),
        title: "Server profiles".to_string(),
        full_screen: false,
        controls,
        on_open: Rc::new(|_| {}),
        on_close: Rc::new(|_| {}),
    }
}

/// Register the server settings menus plus the linked rotation submenu
/// (donor `registerServerSettingsMenu`).
pub fn register_server_settings_menu(
    controller: &Rc<RefCell<NativeUiController>>,
    host: HostServerSettingsUi,
) -> SettingsMenus {
    let rotation_write = host.write_setting.clone();
    let rotation_root = register_rotation_menu_tracked(
        controller,
        ServerRotationBindings {
            read: host.bindings.clone(),
            write: Rc::new(move |value| rotation_write(MAP_ROTATION_SETTING_ID, value).map(|_| ())),
        },
    );
    let ids = ServerMenuIds {
        root: menu_id(ROOT_MENU_ID),
        detail: menu_id(DETAIL_MENU_ID),
        profiles: menu_id(PROFILES_MENU_ID),
        rotation: rotation_root,
    };
    let inputs = ServerMenuInputs {
        host,
        controller: Rc::clone(controller),
        seat: controller.borrow().seat(),
        state: Rc::new(RefCell::new(ServerSettingsMenuState::default())),
    };
    let root_ids = ids.clone();
    let root_inputs = inputs.clone();
    controller.borrow_mut().register(
        ids.root.clone(),
        Rc::new(move || build_server_root_menu(&root_ids, &root_inputs)),
    );
    let detail_ids = ids.clone();
    let detail_inputs = inputs.clone();
    controller.borrow_mut().register(
        ids.detail.clone(),
        Rc::new(move || build_server_detail_menu(&detail_ids, &detail_inputs)),
    );
    let profiles_ids = ids.clone();
    let profiles_inputs = inputs.clone();
    controller.borrow_mut().register(
        ids.profiles.clone(),
        Rc::new(move || build_server_profiles_menu(&profiles_ids, &profiles_inputs)),
    );
    SettingsMenus::new(
        controller,
        ids.root.clone(),
        vec![
            ids.root.clone(),
            ids.detail.clone(),
            ids.profiles.clone(),
            ids.rotation.clone(),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    use super::super::rotation::ServerSettingDefView;
    use crate::ui::common::controller::headless_options;
    use crate::ui::types::UiChoice;

    fn def(id: &str, label: &str, kind: ServerSettingKindView) -> ServerSettingDefView {
        ServerSettingDefView {
            id: id.to_string(),
            label: label.to_string(),
            description: format!("Description of {label}."),
            default_value: "default".to_string(),
            kind,
        }
    }

    fn binding(
        definition: ServerSettingDefView,
        desired: &str,
        effective: &str,
        apply_at: ServerApplyAt,
    ) -> ServerBindingView {
        ServerBindingView {
            definition,
            desired: desired.to_string(),
            effective: effective.to_string(),
            pending: desired != effective,
            apply_at,
        }
    }

    struct FakeHost {
        bindings: Vec<ServerBindingView>,
        writes: Vec<(String, String)>,
        fail_with: Option<String>,
    }

    struct FakeStore {
        files: HashSet<String>,
        payloads: std::collections::HashMap<String, String>,
        fail_with: Option<String>,
    }

    impl FakeStore {
        fn new() -> Self {
            Self {
                files: HashSet::new(),
                payloads: std::collections::HashMap::new(),
                fail_with: None,
            }
        }
    }

    impl ServerProfileStore for FakeStore {
        fn read(&self, name: &str) -> Option<String> {
            self.payloads.get(name).cloned()
        }

        fn write(&mut self, name: &str, value: &str) -> Result<(), String> {
            if let Some(message) = self.fail_with.clone() {
                return Err(message);
            }
            self.files.insert(name.to_string());
            self.payloads.insert(name.to_string(), value.to_string());
            Ok(())
        }
    }

    struct Harness {
        controller: Rc<RefCell<NativeUiController>>,
        ids: ServerMenuIds,
        inputs: ServerMenuInputs,
        seat: SeatId,
        fake: Rc<RefCell<FakeHost>>,
        store: Rc<RefCell<FakeStore>>,
    }

    impl Harness {
        fn new(bindings: Vec<ServerBindingView>) -> Self {
            let owner = IdentityOwner::create("server-test").expect("test owner");
            let seat = owner.seat(0);
            let controller = Rc::new(RefCell::new(NativeUiController::new(headless_options(seat.clone()))));
            let fake = Rc::new(RefCell::new(FakeHost {
                bindings,
                writes: Vec::new(),
                fail_with: None,
            }));
            let store = Rc::new(RefCell::new(FakeStore::new()));
            let read_fake = Rc::clone(&fake);
            let write_fake = Rc::clone(&fake);
            let host = HostServerSettingsUi {
                bindings: Rc::new(move || read_fake.borrow().bindings.clone()),
                write_setting: Rc::new(move |id, value| {
                    let mut fake = write_fake.borrow_mut();
                    if let Some(message) = fake.fail_with.clone() {
                        return Err(message);
                    }
                    fake.writes.push((id.to_string(), value.to_string()));
                    for binding in &mut fake.bindings {
                        if binding.definition.id == id {
                            binding.desired = value.to_string();
                            binding.pending = binding.desired != binding.effective;
                        }
                    }
                    Ok(value.to_string())
                }),
                store: Rc::clone(&store) as Rc<RefCell<dyn ServerProfileStore>>,
            };
            let ids = ServerMenuIds {
                root: menu_id(ROOT_MENU_ID),
                detail: menu_id(DETAIL_MENU_ID),
                profiles: menu_id(PROFILES_MENU_ID),
                rotation: menu_id("menu:server:rotation"),
            };
            let inputs = ServerMenuInputs {
                host,
                controller: Rc::clone(&controller),
                seat: seat.clone(),
                state: Rc::new(RefCell::new(ServerSettingsMenuState::default())),
            };
            Self {
                controller,
                ids,
                inputs,
                seat,
                fake,
                store,
            }
        }

        fn root(&self) -> UiMenu {
            build_server_root_menu(&self.ids, &self.inputs)
        }

        fn detail(&self) -> UiMenu {
            build_server_detail_menu(&self.ids, &self.inputs)
        }

        fn profiles(&self) -> UiMenu {
            build_server_profiles_menu(&self.ids, &self.inputs)
        }

        fn control(menu: &UiMenu, id: &str) -> UiControl {
            menu.controls
                .iter()
                .find(|control| control.id.as_str() == id)
                .unwrap_or_else(|| panic!("missing control: {id}"))
                .clone()
        }

        fn has_control(menu: &UiMenu, id: &str) -> bool {
            menu.controls.iter().any(|control| control.id.as_str() == id)
        }

        fn label(menu: &UiMenu, id: &str) -> String {
            Self::control(menu, id).label.clone()
        }

        fn enabled(menu: &UiMenu, id: &str) -> bool {
            Self::control(menu, id).enabled
        }

        fn activate(&self, menu: &UiMenu, id: &str) {
            match &Self::control(menu, id).kind {
                UiControlKind::Button { on_activate } => on_activate(self.seat.clone()),
                other => panic!("control is not a button: {id} ({other:?})"),
            }
        }

        fn change_text(&self, menu: &UiMenu, id: &str, value: &str) {
            match &Self::control(menu, id).kind {
                UiControlKind::TextEntry { on_change, .. } => on_change(self.seat.clone(), value),
                other => panic!("control is not a text entry: {id} ({other:?})"),
            }
        }

        fn toggle(&self, menu: &UiMenu, id: &str, value: bool) {
            match &Self::control(menu, id).kind {
                UiControlKind::Toggle { on_change, .. } => on_change(self.seat.clone(), value),
                other => panic!("control is not a toggle: {id} ({other:?})"),
            }
        }

        fn select(&self, menu: &UiMenu, id: &str, value: &str) {
            match &Self::control(menu, id).kind {
                UiControlKind::Choice { on_select, .. } => on_select(self.seat.clone(), value),
                other => panic!("control is not a choice: {id} ({other:?})"),
            }
        }

        fn select_binding(&self, id: &str) {
            let menu = self.root();
            self.activate(&menu, &format!("ui:server:{id}"));
        }
    }

    fn paging_bindings() -> Vec<ServerBindingView> {
        (0..8)
            .map(|index| {
                let id = format!("server:setting-{index}");
                let pending = index == 1;
                binding(
                    def(&id, &format!("Setting {index}"), ServerSettingKindView::Toggle),
                    if pending { "1" } else { "0" },
                    "0",
                    ServerApplyAt::Live,
                )
            })
            .collect()
    }

    fn kind_bindings() -> Vec<ServerBindingView> {
        vec![
            binding(
                ServerSettingDefView {
                    default_value: "1".to_string(),
                    description: "A toggle setting with a short description.".to_string(),
                    ..def("server:toggle", "Toggle", ServerSettingKindView::Toggle)
                },
                "1",
                "1",
                ServerApplyAt::Live,
            ),
            binding(
                ServerSettingDefView {
                    default_value: "fast".to_string(),
                    ..def(
                        "server:mode",
                        "Mode",
                        ServerSettingKindView::Choice {
                            choices: vec![
                                UiChoice {
                                    id: "fast".to_string(),
                                    label: "Fast".to_string(),
                                },
                                UiChoice {
                                    id: "slow".to_string(),
                                    label: "Slow".to_string(),
                                },
                            ],
                        },
                    )
                },
                "slow",
                "fast",
                ServerApplyAt::NextMatch,
            ),
            binding(
                ServerSettingDefView {
                    default_value: "8".to_string(),
                    ..def(
                        "server:limit",
                        "Limit",
                        ServerSettingKindView::Slider {
                            minimum: 1.0,
                            maximum: 16.0,
                            integer: true,
                        },
                    )
                },
                "8",
                "8",
                ServerApplyAt::NextMap,
            ),
            binding(
                ServerSettingDefView {
                    default_value: "host".to_string(),
                    ..def(
                        "server:name",
                        "Name",
                        ServerSettingKindView::TextEntry { maximum_length: 12 },
                    )
                },
                "custom",
                "custom",
                ServerApplyAt::Restart,
            ),
            binding(
                def("server:mystery", "Mystery", ServerSettingKindView::Other),
                "x",
                "x",
                ServerApplyAt::Live,
            ),
            binding(
                ServerSettingDefView {
                    default_value: String::new(),
                    description: "Ordered map names.".to_string(),
                    ..def(
                        MAP_ROTATION_SETTING_ID,
                        "Map rotation",
                        ServerSettingKindView::TextEntry { maximum_length: 2048 },
                    )
                },
                "a b",
                "a b",
                ServerApplyAt::Live,
            ),
        ]
    }

    #[test]
    fn timing_covers_pending_and_apply_points() {
        let live = || def("server:x", "X", ServerSettingKindView::Toggle);
        let cases = [
            (binding(live(), "0", "0", ServerApplyAt::Live), "Applies during play"),
            (
                binding(live(), "1", "0", ServerApplyAt::Live),
                "Pending: applies on next source update",
            ),
            (
                binding(live(), "0", "0", ServerApplyAt::NextMatch),
                "Changes apply on next match",
            ),
            (
                binding(live(), "1", "0", ServerApplyAt::NextMatch),
                "Pending: applies on next match",
            ),
            (
                binding(live(), "0", "0", ServerApplyAt::NextMap),
                "Changes apply on next map",
            ),
            (
                binding(live(), "1", "0", ServerApplyAt::NextMap),
                "Pending: applies on next map",
            ),
            (
                binding(live(), "0", "0", ServerApplyAt::Restart),
                "Changes apply on server restart",
            ),
            (
                binding(live(), "1", "0", ServerApplyAt::Restart),
                "Pending: applies on server restart",
            ),
        ];
        for (binding, expected) in cases {
            assert_eq!(server_setting_timing(&binding), expected);
        }
    }

    #[test]
    fn wrap_lines_matches_donor_width() {
        assert!(wrap_lines("").is_empty());
        assert_eq!(wrap_lines("short"), vec!["short".to_string()]);
        let words = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor";
        assert_eq!(
            wrap_lines(words),
            vec![
                "lorem ipsum dolor sit amet consectetur adipiscing elit sed do".to_string(),
                "eiusmod tempor".to_string(),
            ],
        );
        let long_word = "w".repeat(61);
        assert_eq!(wrap_lines(&long_word), vec![String::new(), long_word]);
    }

    #[test]
    fn profile_names_follow_donor_pattern() {
        for valid in ["default", "a", "Z", "9", "league-night_2", &"a".repeat(32)] {
            assert!(is_valid_profile_name(valid), "name rejected: {valid:?}");
        }
        for invalid in [
            "",
            "-lead",
            "_lead",
            "has space",
            "a/b",
            "a.b",
            "name!",
            &"a".repeat(33),
        ] {
            assert!(!is_valid_profile_name(invalid), "name accepted: {invalid:?}");
        }
    }

    #[test]
    fn profile_payload_round_trips_escapes() {
        let profile: ProfileData = vec![
            ("server:plain".to_string(), "value".to_string()),
            ("server:empty".to_string(), String::new()),
            ("we=ird\\id".to_string(), "a=b\\c\nd\re".to_string()),
        ];
        let payload = encode_server_profile(&profile).expect("encode");
        assert!(payload.starts_with("server-profile 1\n"));
        assert!(payload.ends_with('\n'));
        assert_eq!(decode_server_profile(&payload).expect("decode"), profile);
        assert_eq!(
            decode_server_profile(payload.trim_end_matches('\n')).expect("no trailing newline"),
            profile
        );
        let empty = encode_server_profile(&[]).expect("encode empty");
        assert_eq!(empty, "server-profile 1\n");
        assert_eq!(decode_server_profile(&empty).expect("decode empty"), ProfileData::new());
    }

    #[test]
    fn profile_payload_rejects_malformed_input() {
        assert!(encode_server_profile(&[(String::new(), "v".to_string())]).is_err());
        assert!(
            encode_server_profile(&[("a".to_string(), "1".to_string()), ("a".to_string(), "2".to_string())]).is_err()
        );
        assert_eq!(
            decode_server_profile("server-profile 1").expect("header-only"),
            ProfileData::new()
        );
        for payload in [
            "",
            "{}\n",
            "server-profile 2\na=b\n",
            "server-profile 1\nno-separator\n",
            "server-profile 1\n\\x=y\n",
            "server-profile 1\ntrailing\\\n",
            "server-profile 1\n\\=v\n",
            "server-profile 1\na=1\na=2\n",
            "server-profile 1\na=1\n\n",
        ] {
            assert!(decode_server_profile(payload).is_err(), "payload accepted: {payload:?}");
        }
    }

    #[test]
    fn capture_uses_desired_values() {
        let bindings = kind_bindings();
        let captured = capture_server_profile(&bindings);
        assert_eq!(captured.len(), bindings.len());
        for (binding, (id, value)) in bindings.iter().zip(captured.iter()) {
            assert_eq!(id, &binding.definition.id);
            assert_eq!(value, &binding.desired);
        }
    }

    #[test]
    fn apply_writes_in_order_and_rejects_duplicates_first() {
        let writes = Rc::new(RefCell::new(Vec::new()));
        let write_writes = Rc::clone(&writes);
        let profile: ProfileData = vec![("a".to_string(), "1".to_string()), ("b".to_string(), "2".to_string())];
        apply_server_profile(&profile, &move |id, value| {
            write_writes.borrow_mut().push((id.to_string(), value.to_string()));
            Ok(format!("normalized-{value}"))
        })
        .expect("apply");
        assert_eq!(*writes.borrow(), profile);

        writes.borrow_mut().clear();
        let duplicates: ProfileData = vec![("a".to_string(), "1".to_string()), ("a".to_string(), "2".to_string())];
        let write_writes = Rc::clone(&writes);
        let error = apply_server_profile(&duplicates, &move |id, value| {
            write_writes.borrow_mut().push((id.to_string(), value.to_string()));
            Ok(value.to_string())
        })
        .expect_err("duplicate ids");
        assert_eq!(error, "Duplicate server profile setting a");
        assert!(writes.borrow().is_empty());

        writes.borrow_mut().clear();
        let write_writes = Rc::clone(&writes);
        let error = apply_server_profile(&profile, &move |id, value| {
            write_writes.borrow_mut().push((id.to_string(), value.to_string()));
            if id == "b" {
                return Err("Owner refused b.".to_string());
            }
            Ok(value.to_string())
        })
        .expect_err("owner rejection");
        assert_eq!(error, "Owner refused b.");
        assert_eq!(writes.borrow().len(), 2);
    }

    #[test]
    fn root_pages_seven_per_page_with_pending_markers() {
        let harness = Harness::new(paging_bindings());
        let menu = harness.root();
        assert_eq!(menu.title, "Server settings 1/2");
        assert_eq!(Harness::label(&menu, "ui:server:server:setting-0"), "Setting 0: 0");
        assert_eq!(
            Harness::label(&menu, "ui:server:server:setting-1"),
            "Setting 1: 1 (pending)"
        );
        assert!(!Harness::has_control(&menu, "ui:server:server:setting-7"));
        assert!(!Harness::enabled(&menu, "ui:server:previous"));
        assert!(Harness::enabled(&menu, "ui:server:next"));
        harness.activate(&menu, "ui:server:next");
        let menu = harness.root();
        assert_eq!(menu.title, "Server settings 2/2");
        assert_eq!(Harness::label(&menu, "ui:server:server:setting-7"), "Setting 7: 0");
        assert!(Harness::enabled(&menu, "ui:server:previous"));
        assert!(!Harness::enabled(&menu, "ui:server:next"));
        harness.activate(&menu, "ui:server:previous");
        assert_eq!(harness.root().title, "Server settings 1/2");
    }

    #[test]
    fn root_row_opens_detail_and_seeds_draft() {
        use crate::ui::types::SeatUiController as _;

        let harness = Harness::new(kind_bindings());
        let root_ids = harness.ids.clone();
        let root_inputs = harness.inputs.clone();
        harness.controller.borrow_mut().register(
            harness.ids.root.clone(),
            Rc::new(move || build_server_root_menu(&root_ids, &root_inputs)),
        );
        let detail_ids = harness.ids.clone();
        let detail_inputs = harness.inputs.clone();
        harness.controller.borrow_mut().register(
            harness.ids.detail.clone(),
            Rc::new(move || build_server_detail_menu(&detail_ids, &detail_inputs)),
        );
        harness
            .controller
            .borrow_mut()
            .open_menu(&harness.ids.root)
            .expect("open root");
        harness.select_binding("server:mode");
        assert_eq!(harness.inputs.state.borrow().selected.as_deref(), Some("server:mode"));
        assert_eq!(harness.inputs.state.borrow().draft, "slow");
        assert_eq!(
            harness.controller.borrow().active_menu(),
            Some(harness.ids.detail.clone())
        );
        let detail = harness.detail();
        assert_eq!(detail.title, "Mode");
    }

    #[test]
    fn root_row_links_rotation_binding_to_rotation_menu() {
        use crate::ui::types::SeatUiController as _;

        let harness = Harness::new(kind_bindings());
        let root_ids = harness.ids.clone();
        let root_inputs = harness.inputs.clone();
        harness.controller.borrow_mut().register(
            harness.ids.root.clone(),
            Rc::new(move || build_server_root_menu(&root_ids, &root_inputs)),
        );
        let rotation_id = harness.ids.rotation.clone();
        harness.controller.borrow_mut().register(
            harness.ids.rotation.clone(),
            Rc::new(move || UiMenu {
                scroll: None,
                id: rotation_id.clone(),
                title: "Map rotation".to_string(),
                full_screen: false,
                controls: Vec::new(),
                on_open: Rc::new(|_| {}),
                on_close: Rc::new(|_| {}),
            }),
        );
        harness
            .controller
            .borrow_mut()
            .open_menu(&harness.ids.root)
            .expect("open root");
        harness.select_binding(MAP_ROTATION_SETTING_ID);
        assert_eq!(
            harness.controller.borrow().active_menu(),
            Some(harness.ids.rotation.clone())
        );
    }

    #[test]
    fn detail_renders_toggle_choice_slider_text_and_other() {
        let harness = Harness::new(kind_bindings());

        harness.select_binding("server:toggle");
        let detail = harness.detail();
        match &Harness::control(&detail, "ui:server:value").kind {
            UiControlKind::Toggle { checked, .. } => assert!(*checked),
            other => panic!("toggle detail is not a toggle ({other:?})"),
        }
        harness.toggle(&detail, "ui:server:value", false);
        assert_eq!(harness.inputs.state.borrow().draft, "0");
        assert_eq!(
            Harness::label(&detail, "ui:server:info-desired"),
            "Desired: 1   Effective: 1"
        );
        assert_eq!(Harness::label(&detail, "ui:server:info-default"), "Default: 1");
        assert_eq!(Harness::label(&detail, "ui:server:info-timing"), "Applies during play");

        harness.select_binding("server:mode");
        let detail = harness.detail();
        match &Harness::control(&detail, "ui:server:value").kind {
            UiControlKind::Choice { choices, selected, .. } => {
                assert_eq!(choices.len(), 2);
                assert_eq!(selected.as_deref(), Some("slow"));
            }
            other => panic!("choice detail is not a choice ({other:?})"),
        }
        harness.select(&detail, "ui:server:value", "fast");
        assert_eq!(harness.inputs.state.borrow().draft, "fast");
        assert_eq!(
            Harness::label(&detail, "ui:server:info-timing"),
            "Pending: applies on next match"
        );

        harness.select_binding("server:limit");
        let detail = harness.detail();
        match &Harness::control(&detail, "ui:server:value").kind {
            UiControlKind::TextEntry {
                text, maximum_length, ..
            } => {
                assert_eq!(text, "8");
                assert_eq!(*maximum_length, DRAFT_MAXIMUM_LENGTH);
            }
            other => panic!("slider detail is not a text draft ({other:?})"),
        }
        assert_eq!(
            Harness::label(&detail, "ui:server:info-range"),
            "Whole numbers from 1 to 16"
        );

        harness.select_binding("server:name");
        let detail = harness.detail();
        match &Harness::control(&detail, "ui:server:value").kind {
            UiControlKind::TextEntry { maximum_length, .. } => assert_eq!(*maximum_length, 12),
            other => panic!("text detail is not a text entry ({other:?})"),
        }

        harness.select_binding("server:mystery");
        let detail = harness.detail();
        match &Harness::control(&detail, "ui:server:value").kind {
            UiControlKind::TextEntry { maximum_length, .. } => assert_eq!(*maximum_length, DRAFT_MAXIMUM_LENGTH),
            other => panic!("other detail is not a text draft ({other:?})"),
        }
    }

    #[test]
    fn detail_apply_default_and_missing() {
        let harness = Harness::new(kind_bindings());
        harness.select_binding("server:toggle");
        harness.toggle(&harness.detail(), "ui:server:value", false);
        harness.activate(&harness.detail(), "ui:server:apply");
        assert_eq!(
            harness.fake.borrow().writes,
            vec![("server:toggle".to_string(), "0".to_string())]
        );
        assert_eq!(harness.inputs.state.borrow().draft, "0");
        assert!(harness.inputs.state.borrow().error.is_empty());

        harness.toggle(&harness.detail(), "ui:server:value", true);
        harness.activate(&harness.detail(), "ui:server:default");
        assert_eq!(harness.inputs.state.borrow().draft, "1");
        assert!(harness.inputs.state.borrow().error.is_empty());

        harness.fake.borrow_mut().fail_with = Some("Owner is offline.".to_string());
        harness.activate(&harness.detail(), "ui:server:apply");
        assert_eq!(
            Harness::label(&harness.detail(), "ui:server:info-error-0"),
            "Owner is offline."
        );

        let stale = harness.detail();
        harness.fake.borrow_mut().bindings.clear();
        harness.activate(&stale, "ui:server:apply");
        let detail = harness.detail();
        assert_eq!(detail.title, "Server setting");
        assert_eq!(Harness::label(&detail, "ui:server:info-missing"), MISSING_SETTING_ERROR);
        assert!(!Harness::has_control(&detail, "ui:server:apply"));
        assert_eq!(Harness::label(&detail, "ui:server:info-error-0"), MISSING_SETTING_ERROR);
    }

    #[test]
    fn profiles_validate_save_and_load() {
        let harness = Harness::new(kind_bindings());
        harness.change_text(&harness.profiles(), "ui:server:profile-name", "bad name!");
        harness.activate(&harness.profiles(), "ui:server:save");
        assert_eq!(
            Harness::label(&harness.profiles(), "ui:server:info-profile-message-0"),
            PROFILE_NAME_ERROR
        );
        assert!(harness.store.borrow().payloads.is_empty());

        harness.change_text(&harness.profiles(), "ui:server:profile-name", "league-night");
        harness.activate(&harness.profiles(), "ui:server:save");
        assert_eq!(
            Harness::label(&harness.profiles(), "ui:server:info-profile-message-0"),
            "Saved league-night."
        );
        let path = server_profile_path("league-night");
        assert_eq!(path, "servers/league-night.json");
        let payload = harness
            .store
            .borrow()
            .payloads
            .get(&path)
            .cloned()
            .expect("saved payload");
        let decoded = decode_server_profile(&payload).expect("saved payload decodes");
        assert_eq!(decoded, capture_server_profile(&harness.fake.borrow().bindings));

        harness.fake.borrow_mut().writes.clear();
        harness.activate(&harness.profiles(), "ui:server:load");
        assert_eq!(
            Harness::label(&harness.profiles(), "ui:server:info-profile-message-0"),
            "Loaded league-night."
        );
        assert_eq!(harness.fake.borrow().writes.len(), harness.fake.borrow().bindings.len());

        harness.change_text(&harness.profiles(), "ui:server:profile-name", "missing");
        harness.activate(&harness.profiles(), "ui:server:load");
        assert_eq!(
            Harness::label(&harness.profiles(), "ui:server:info-profile-message-0"),
            "Profile missing does not exist."
        );
    }

    #[test]
    fn profiles_surface_store_failures() {
        let harness = Harness::new(kind_bindings());
        harness.store.borrow_mut().fail_with = Some("Disk is full.".to_string());
        harness.activate(&harness.profiles(), "ui:server:save");
        assert_eq!(
            Harness::label(&harness.profiles(), "ui:server:info-profile-message-0"),
            "Disk is full."
        );
    }

    #[test]
    fn register_tracks_all_menus_and_dispose_unregisters() {
        use crate::ui::types::SeatUiController as _;

        let owner = IdentityOwner::create("server-register-test").expect("test owner");
        let seat = owner.seat(0);
        let controller = Rc::new(RefCell::new(NativeUiController::new(headless_options(seat))));
        let fake = Rc::new(RefCell::new(FakeHost {
            bindings: kind_bindings(),
            writes: Vec::new(),
            fail_with: None,
        }));
        let store: Rc<RefCell<dyn ServerProfileStore>> = Rc::new(RefCell::new(FakeStore::new()));
        let read_fake = Rc::clone(&fake);
        let write_fake = Rc::clone(&fake);
        let host = HostServerSettingsUi {
            bindings: Rc::new(move || read_fake.borrow().bindings.clone()),
            write_setting: Rc::new(move |id, value| {
                write_fake.borrow_mut().writes.push((id.to_string(), value.to_string()));
                Ok(value.to_string())
            }),
            store,
        };
        let menus = register_server_settings_menu(&controller, host);
        assert_eq!(menus.root.as_str(), ROOT_MENU_ID);
        let detail = menu_id(DETAIL_MENU_ID);
        let profiles = menu_id(PROFILES_MENU_ID);
        let rotation = menu_id("menu:server:rotation");
        assert!(controller.borrow().is_registered(&menus.root));
        assert!(controller.borrow().is_registered(&detail));
        assert!(controller.borrow().is_registered(&profiles));
        assert!(controller.borrow().is_registered(&rotation));
        controller.borrow_mut().open_menu(&menus.root).expect("open root");
        menus.dispose();
        assert!(!controller.borrow().is_registered(&menu_id(ROOT_MENU_ID)));
        assert!(!controller.borrow().is_registered(&detail));
        assert!(!controller.borrow().is_registered(&profiles));
        assert!(!controller.borrow().is_registered(&rotation));
    }
}
