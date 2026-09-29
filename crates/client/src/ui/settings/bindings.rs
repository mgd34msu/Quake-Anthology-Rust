//! Seat-owned key binding menus.
//!
//! Ported from the TypeScript donor's `src/ui/settings/bindings.ts`
//! (`registerBindingMenus`) plus the physical-input half of
//! `src/input/bindings.ts` (`physicalInputName`, `controllerButtons`) and
//! `src/input/keys.ts` (`keynumToString`). Binding edits stay seat-owned
//! and conflicts require explicit replacement.

use std::cell::{Ref, RefCell, RefMut};
use std::rc::Rc;

use qa_core::identity::SeatId;

use crate::input::{
    quake_mouse_button, AxisDirection, BindingTable, ControllerAxis, InputAction, InputBinding, InputBindingTarget,
    PhysicalInput,
};
use crate::text::draw2d::Rect;
use crate::ui::common::controller::NativeUiController;
use crate::ui::common::layout::{menu_row, MenuRowOptions};
use crate::ui::types::{
    ListRowAction, SeatUiController, UiControl, UiControlId, UiControlKind, UiListRow, UiMenu, UiMenuId,
};

/// Default binding reset behind a settings menu.
pub trait BindingReset {
    /// Whether defaults are available to restore.
    fn available(&self) -> bool;
    /// Restore defaults.
    fn reset(&mut self);
}

/// Extra matcher for equivalent command spellings.
pub type BindingMatcher = Rc<dyn Fn(&InputBindingTarget) -> bool>;

/// One bindable action row.
#[derive(Clone)]
pub struct BindingAction {
    /// Stable action identity.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Canonical target bound by the row.
    pub target: InputBindingTarget,
    /// Extra matcher for equivalent command spellings.
    pub matches: Option<BindingMatcher>,
}

impl std::fmt::Debug for BindingAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BindingAction")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("target", &self.target)
            .field("matches", &self.matches.is_some())
            .finish()
    }
}

/// Donor `InputAction` text for one Rust action.
fn input_action_as_str(action: &InputAction) -> &'static str {
    match action {
        InputAction::Attack => "attack",
        InputAction::Jump => "jump",
        InputAction::Forward => "forward",
        InputAction::Back => "back",
        InputAction::MoveLeft => "move-left",
        InputAction::MoveRight => "move-right",
        InputAction::MoveUp => "move-up",
        InputAction::MoveDown => "move-down",
        InputAction::Use => "use",
        InputAction::Crouch => "crouch",
        InputAction::Walk => "walk",
        InputAction::Scores => "scores",
        InputAction::NextWeapon => "next-weapon",
        InputAction::PreviousWeapon => "previous-weapon",
        InputAction::Menu => "menu",
    }
}

/// Donor `keyNames` lookup for one key number.
fn key_name(code: i32) -> Option<String> {
    match code {
        9 => return Some("TAB".to_string()),
        13 => return Some("ENTER".to_string()),
        27 => return Some("ESCAPE".to_string()),
        32 => return Some("SPACE".to_string()),
        127 => return Some("BACKSPACE".to_string()),
        132 => return Some("UPARROW".to_string()),
        133 => return Some("DOWNARROW".to_string()),
        134 => return Some("LEFTARROW".to_string()),
        135 => return Some("RIGHTARROW".to_string()),
        136 => return Some("ALT".to_string()),
        137 => return Some("CTRL".to_string()),
        138 => return Some("SHIFT".to_string()),
        128 => return Some("COMMAND".to_string()),
        129 => return Some("CAPSLOCK".to_string()),
        139 => return Some("INS".to_string()),
        140 => return Some("DEL".to_string()),
        141 => return Some("PGDN".to_string()),
        142 => return Some("PGUP".to_string()),
        143 => return Some("HOME".to_string()),
        144 => return Some("END".to_string()),
        184 => return Some("MWHEELUP".to_string()),
        183 => return Some("MWHEELDOWN".to_string()),
        160 => return Some("KP_HOME".to_string()),
        161 => return Some("KP_UPARROW".to_string()),
        162 => return Some("KP_PGUP".to_string()),
        163 => return Some("KP_LEFTARROW".to_string()),
        164 => return Some("KP_5".to_string()),
        165 => return Some("KP_RIGHTARROW".to_string()),
        166 => return Some("KP_END".to_string()),
        167 => return Some("KP_DOWNARROW".to_string()),
        168 => return Some("KP_PGDN".to_string()),
        169 => return Some("KP_ENTER".to_string()),
        170 => return Some("KP_INS".to_string()),
        171 => return Some("KP_DEL".to_string()),
        172 => return Some("KP_SLASH".to_string()),
        173 => return Some("KP_MINUS".to_string()),
        174 => return Some("KP_PLUS".to_string()),
        175 => return Some("KP_NUMLOCK".to_string()),
        176 => return Some("KP_STAR".to_string()),
        177 => return Some("KP_EQUALS".to_string()),
        131 => return Some("PAUSE".to_string()),
        59 => return Some("SEMICOLON".to_string()),
        _ => {}
    }
    if (145..=156).contains(&code) {
        return Some(format!("F{}", code - 144));
    }
    if (178..=182).contains(&code) {
        return Some(format!("MOUSE{}", code - 177));
    }
    if (185..=216).contains(&code) {
        return Some(format!("JOY{}", code - 184));
    }
    if (217..=232).contains(&code) {
        return Some(format!("AUX{}", code - 216));
    }
    if (256..=271).contains(&code) {
        return Some(format!("AUX{}", code - 256 + 17));
    }
    None
}

/// Donor `keynumToString` for one key number.
fn keynum_to_string(code: i32) -> String {
    if code == -1 {
        return "<KEY NOT FOUND>".to_string();
    }
    if !(0..=271).contains(&code) {
        return "<OUT OF RANGE>".to_string();
    }
    if code > 32 && code < 127 && code != 34 && code != 59 {
        let byte = code as u8;
        return (byte as char).to_string();
    }
    match key_name(code) {
        Some(name) => name,
        None => format!("0x{code:02x}"),
    }
}

/// Donor `controllerButtons` label for one button index.
fn controller_button_label(button: i32) -> String {
    match button {
        0 => "A",
        1 => "B",
        2 => "X",
        3 => "Y",
        4 => "Back",
        5 => "Guide",
        6 => "Start",
        7 => "Left stick press",
        8 => "Right stick press",
        9 => "Left shoulder",
        10 => "Right shoulder",
        11 => "D-pad up",
        12 => "D-pad down",
        13 => "D-pad left",
        14 => "D-pad right",
        15 => "Miscellaneous",
        16 => "Paddle 1",
        17 => "Paddle 2",
        18 => "Paddle 3",
        19 => "Paddle 4",
        20 => "Touchpad",
        _ => return format!("Button {}", button + 1),
    }
    .to_string()
}

/// Display label for one physical input.
#[must_use]
pub fn physical_input_label(input: &PhysicalInput) -> String {
    match input {
        PhysicalInput::Key(code) => keynum_to_string(*code),
        PhysicalInput::MouseButton(button) => {
            format!("MOUSE{}", quake_mouse_button(*button))
        }
        PhysicalInput::ControllerButton { device, button } => {
            format!("Pad {} {}", device + 1, controller_button_label(*button))
        }
        PhysicalInput::ControllerAxis {
            device,
            axis,
            direction,
        } => {
            let prefix = format!("Pad {}", device + 1);
            let positive = *direction == AxisDirection::Positive;
            match axis {
                ControllerAxis::LeftX => {
                    format!("{prefix} Left stick {}", if positive { "right" } else { "left" })
                }
                ControllerAxis::LeftY => {
                    format!("{prefix} Left stick {}", if positive { "down" } else { "up" })
                }
                ControllerAxis::RightX => {
                    format!("{prefix} Right stick {}", if positive { "right" } else { "left" })
                }
                ControllerAxis::RightY => {
                    format!("{prefix} Right stick {}", if positive { "down" } else { "up" })
                }
                ControllerAxis::LeftTrigger => {
                    format!("{prefix} Left trigger{}", if positive { "" } else { " released" })
                }
                ControllerAxis::RightTrigger => {
                    format!("{prefix} Right trigger{}", if positive { "" } else { " released" })
                }
            }
        }
    }
}

/// Whether a bound target belongs to one action row.
#[must_use]
pub fn binding_matches_action(target: &InputBindingTarget, action: &BindingAction) -> bool {
    let same = match (target, &action.target) {
        (InputBindingTarget::Action(current), InputBindingTarget::Action(want)) => current == want,
        (InputBindingTarget::Command(current), InputBindingTarget::Command(want)) => current == want,
        _ => false,
    };
    if same {
        return true;
    }
    match action.matches {
        Some(ref matches) => matches(target),
        None => false,
    }
}

/// Seat-owned binding storage behind the menus.
pub trait BindingStore {
    /// Owning seat.
    fn seat(&self) -> SeatId;
    /// All bindings.
    fn bindings(&self) -> Vec<InputBinding>;
    /// Target bound to an input, if any.
    fn binding(&self, input: &PhysicalInput) -> Option<InputBindingTarget>;
    /// Bind an input, replacing any previous binding.
    fn bind(&mut self, binding: InputBinding);
    /// Remove a binding.
    fn unbind(&mut self, input: &PhysicalInput);
}

/// Binding storage pairing one seat with one table.
#[derive(Debug, Clone)]
pub struct SeatBindingStore {
    /// Owning seat.
    pub seat: SeatId,
    /// Binding table.
    pub table: BindingTable,
}

impl SeatBindingStore {
    /// Empty storage for one seat.
    #[must_use]
    pub fn new(seat: SeatId) -> Self {
        Self {
            seat,
            table: BindingTable::new(),
        }
    }
}

impl BindingStore for SeatBindingStore {
    fn seat(&self) -> SeatId {
        self.seat.clone()
    }

    fn bindings(&self) -> Vec<InputBinding> {
        self.table.bindings().into_iter().cloned().collect()
    }

    fn binding(&self, input: &PhysicalInput) -> Option<InputBindingTarget> {
        self.table.binding(input).cloned()
    }

    fn bind(&mut self, binding: InputBinding) {
        self.table.bind(binding);
    }

    fn unbind(&mut self, input: &PhysicalInput) {
        self.table.unbind(input);
    }
}

/// Fixed or live action catalog behind the menus.
#[derive(Clone)]
pub enum BindingSource {
    /// Fixed rows.
    Fixed(Vec<BindingAction>),
    /// Rows rebuilt on every menu read.
    Live(Rc<dyn Fn() -> Vec<BindingAction>>),
}

impl std::fmt::Debug for BindingSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BindingSource::Fixed(rows) => f.debug_tuple("Fixed").field(&rows.len()).finish(),
            BindingSource::Live(_) => f.debug_tuple("Live").finish_non_exhaustive(),
        }
    }
}

/// Reset callbacks behind the optional restore menu.
#[derive(Clone)]
pub struct BindingResetHandle {
    /// Whether defaults are available.
    pub available: Rc<dyn Fn() -> bool>,
    /// Restore defaults.
    pub reset: Rc<dyn Fn()>,
}

impl std::fmt::Debug for BindingResetHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BindingResetHandle").finish_non_exhaustive()
    }
}

/// Registered binding menus.
pub struct BindingMenus {
    /// Root menu id.
    pub root: UiMenuId,
    controller: Rc<RefCell<NativeUiController>>,
    menus: Vec<UiMenuId>,
}

impl BindingMenus {
    /// Unregister every binding menu in reverse order.
    pub fn dispose(self) {
        for id in self.menus.iter().rev() {
            let id = id.clone();
            with_controller(&self.controller, move |controller| {
                controller.unregister(&id);
            });
        }
    }
}

impl std::fmt::Debug for BindingMenus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BindingMenus")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

/// One rendered binding row.
#[derive(Debug, Clone)]
struct BindingRow {
    id: String,
    action: BindingAction,
    physical: Option<PhysicalInput>,
    target: InputBindingTarget,
}

/// One pending conflict awaiting explicit replacement.
#[derive(Debug, Clone)]
struct PendingBinding {
    physical: PhysicalInput,
    row: BindingRow,
    previous: InputBindingTarget,
}

/// Mutable menu state shared by every binding factory.
#[derive(Debug, Default)]
struct BindingMenuState {
    query: String,
    selected: Option<String>,
    pending: Option<PendingBinding>,
}

/// Static control id; panics only when a literal is mistyped.
fn control_id(text: &str) -> UiControlId {
    match UiControlId::new(text) {
        Ok(id) => id,
        Err(_) => panic!("static control id is invalid: {text}"),
    }
}

/// Static menu id; panics only when a literal is mistyped.
fn menu_id(text: &str) -> UiMenuId {
    match UiMenuId::new(text) {
        Ok(id) => id,
        Err(_) => panic!("static menu id is invalid: {text}"),
    }
}

/// Run one controller mutation, including re-entrant menu callbacks.
///
/// Button and capture callbacks run while the outer input or draw call
/// already holds the controller borrow; the outer borrow stays suspended
/// while the callback runs and never touches the menu stack afterwards.
fn with_controller(controller: &Rc<RefCell<NativeUiController>>, apply: impl FnOnce(&mut NativeUiController)) {
    match controller.try_borrow_mut() {
        Ok(mut borrowed) => apply(&mut borrowed),
        Err(_) => {
            // SAFETY: the outer borrow is suspended for the duration of this
            // callback and resumes without using stale stack indexes, so the
            // bypassed borrow behaves like a re-entrant menu call.
            unsafe {
                let cell = &*Rc::as_ptr(controller);
                let raw = cell.as_ptr();
                apply(&mut *raw);
            }
        }
    }
}

/// Read the binding store; panics only when a caller holds it borrowed.
fn borrow_input<T>(input: &Rc<RefCell<dyn BindingStore>>, read: impl FnOnce(&dyn BindingStore) -> T) -> T {
    let borrowed: Ref<'_, dyn BindingStore> = match input.try_borrow() {
        Ok(borrowed) => borrowed,
        Err(_) => panic!("binding input is already borrowed"),
    };
    read(&*borrowed)
}

/// Mutate the binding store; panics only when a caller holds it borrowed.
fn borrow_input_mut<T>(input: &Rc<RefCell<dyn BindingStore>>, update: impl FnOnce(&mut dyn BindingStore) -> T) -> T {
    let mut borrowed: RefMut<'_, dyn BindingStore> = match input.try_borrow_mut() {
        Ok(borrowed) => borrowed,
        Err(_) => panic!("binding input is already borrowed"),
    };
    update(&mut *borrowed)
}

/// Read menu state; panics only when a caller holds it borrowed.
fn borrow_state<T>(state: &Rc<RefCell<BindingMenuState>>, read: impl FnOnce(&BindingMenuState) -> T) -> T {
    let borrowed: Ref<'_, BindingMenuState> = match state.try_borrow() {
        Ok(borrowed) => borrowed,
        Err(_) => panic!("binding menu state is already borrowed"),
    };
    read(&borrowed)
}

/// Mutate menu state; panics only when a caller holds it borrowed.
fn borrow_state_mut<T>(state: &Rc<RefCell<BindingMenuState>>, update: impl FnOnce(&mut BindingMenuState) -> T) -> T {
    let mut borrowed: RefMut<'_, BindingMenuState> = match state.try_borrow_mut() {
        Ok(borrowed) => borrowed,
        Err(_) => panic!("binding menu state is already borrowed"),
    };
    update(&mut borrowed)
}

/// JSON-escape one command string for a synthesized custom id.
fn json_escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            control if (control as u32) < 0x20 => {
                escaped.push_str(&format!("\\u{:04x}", control as u32));
            }
            _ => escaped.push(ch),
        }
    }
    escaped
}

/// Donor `JSON.stringify(target)` for a custom row id.
fn target_json(target: &InputBindingTarget) -> String {
    match target {
        InputBindingTarget::Command(text) => {
            format!("{{\"kind\":\"command\",\"text\":\"{}\"}}", json_escape(text))
        }
        InputBindingTarget::Action(action) => {
            format!("{{\"kind\":\"action\",\"action\":\"{}\"}}", input_action_as_str(action))
        }
    }
}

/// Donor `JSON.stringify(physical)` for a bound row id.
fn physical_input_json(input: &PhysicalInput) -> String {
    match input {
        PhysicalInput::Key(code) => format!("{{\"kind\":\"key\",\"code\":{code}}}"),
        PhysicalInput::MouseButton(button) => {
            format!("{{\"kind\":\"mouse-button\",\"button\":{button}}}")
        }
        PhysicalInput::ControllerButton { device, button } => {
            format!("{{\"kind\":\"controller-button\",\"device\":{device},\"button\":{button}}}")
        }
        PhysicalInput::ControllerAxis {
            device,
            axis,
            direction,
        } => {
            let axis = match axis {
                ControllerAxis::LeftX => "left-x",
                ControllerAxis::LeftY => "left-y",
                ControllerAxis::RightX => "right-x",
                ControllerAxis::RightY => "right-y",
                ControllerAxis::LeftTrigger => "left-trigger",
                ControllerAxis::RightTrigger => "right-trigger",
            };
            let direction = match direction {
                AxisDirection::Positive => "positive",
                AxisDirection::Negative => "negative",
            };
            format!(
                "{{\"kind\":\"controller-axis\",\"device\":{device},\"axis\":\"{axis}\",\"direction\":\"{direction}\"}}"
            )
        }
    }
}

/// Catalog rows plus one synthesized row per unmatched binding.
fn available_actions(
    source: &Rc<dyn Fn() -> Vec<BindingAction>>,
    input: &Rc<RefCell<dyn BindingStore>>,
) -> Vec<BindingAction> {
    let mut available = source();
    let bindings = borrow_input(input, |store| store.bindings());
    for binding in &bindings {
        if available
            .iter()
            .any(|action| binding_matches_action(&binding.target, action))
        {
            continue;
        }
        let label = match &binding.target {
            InputBindingTarget::Command(text) => format!("Command: {text}"),
            InputBindingTarget::Action(action) => {
                format!("Action: {}", input_action_as_str(action))
            }
        };
        available.push(BindingAction {
            id: format!("custom:{}", target_json(&binding.target)),
            label,
            target: binding.target.clone(),
            matches: None,
        });
    }
    available
}

/// Label for a bound target, falling back to raw command or action text.
fn target_label(target: &InputBindingTarget, actions: &[BindingAction]) -> String {
    match actions.iter().find(|action| binding_matches_action(target, action)) {
        Some(action) => action.label.clone(),
        None => match target {
            InputBindingTarget::Command(text) => text.clone(),
            InputBindingTarget::Action(action) => input_action_as_str(action).to_string(),
        },
    }
}

/// Filter catalog rows by the current search query.
fn binding_rows(actions: &[BindingAction], bindings: &[InputBinding], query: &str) -> Vec<BindingRow> {
    let lowered = query.to_lowercase();
    let trimmed = lowered.trim();
    let terms: Vec<&str> = if trimmed.is_empty() {
        vec![""]
    } else {
        trimmed.split_whitespace().collect()
    };
    let mut rows = Vec::new();
    for action in actions {
        let matched: Vec<&InputBinding> = bindings
            .iter()
            .filter(|binding| binding_matches_action(&binding.target, action))
            .collect();
        let keys = matched
            .iter()
            .map(|binding| physical_input_label(&binding.input))
            .collect::<Vec<_>>()
            .join(" ");
        let searchable = format!("{} {keys}", action.label).to_lowercase();
        if !terms.iter().all(|term| searchable.contains(term)) {
            continue;
        }
        if matched.is_empty() {
            rows.push(BindingRow {
                id: action.id.clone(),
                action: action.clone(),
                physical: None,
                target: action.target.clone(),
            });
        } else {
            for binding in matched {
                rows.push(BindingRow {
                    id: format!("{}:{}", action.id, physical_input_json(&binding.input)),
                    action: action.clone(),
                    physical: Some(binding.input.clone()),
                    target: binding.target.clone(),
                });
            }
        }
    }
    rows
}

/// Bind one physical input to a row target and select the new row.
fn assign_binding(
    input: &Rc<RefCell<dyn BindingStore>>,
    state: &Rc<RefCell<BindingMenuState>>,
    row: &BindingRow,
    physical: &PhysicalInput,
) {
    borrow_input_mut(input, |store| {
        if let Some(previous) = row.physical.as_ref() {
            store.unbind(previous);
        }
        store.bind(InputBinding {
            input: physical.clone(),
            target: row.target.clone(),
        });
    });
    let selected = format!("{}:{}", row.action.id, physical_input_json(physical));
    borrow_state_mut(state, |menu| {
        menu.selected = Some(selected);
    });
}

/// Capture the next physical input for one row.
fn capture_row(
    controller: &Rc<RefCell<NativeUiController>>,
    input: &Rc<RefCell<dyn BindingStore>>,
    state: &Rc<RefCell<BindingMenuState>>,
    conflict: &UiMenuId,
    row: &BindingRow,
) {
    let captured = row.clone();
    let input = Rc::clone(input);
    let state = Rc::clone(state);
    let controller = Rc::clone(controller);
    let conflict = conflict.clone();
    let target = Rc::clone(&controller);
    with_controller(&target, move |menu| {
        let row = captured.clone();
        let input = Rc::clone(&input);
        let state = Rc::clone(&state);
        let controller = Rc::clone(&controller);
        let conflict = conflict.clone();
        menu.capture_binding(
            Box::new(move |physical: PhysicalInput| {
                let previous = borrow_input(&input, |store| store.binding(&physical));
                match previous {
                    None => assign_binding(&input, &state, &row, &physical),
                    Some(target) if binding_matches_action(&target, &row.action) => {
                        assign_binding(&input, &state, &row, &physical);
                    }
                    Some(previous) => {
                        borrow_state_mut(&state, |menu| {
                            menu.pending = Some(PendingBinding {
                                physical: physical.clone(),
                                row: row.clone(),
                                previous,
                            });
                        });
                        with_controller(&controller, |menu| {
                            let _ = menu.open_menu(&conflict);
                        });
                    }
                }
            }),
            Box::new(|| {}),
        );
    });
}

/// Disabled label button.
fn label_button(id: &str, label: String, rect: Rect) -> UiControl {
    UiControl {
        id: control_id(id),
        label,
        rect,
        enabled: false,
        visible: true,
        kind: UiControlKind::Button {
            on_activate: Rc::new(|_| {}),
        },
    }
}

/// Enabled button with one activation callback.
fn action_button(id: &str, label: &str, rect: Rect, enabled: bool, on_activate: Rc<dyn Fn(SeatId)>) -> UiControl {
    UiControl {
        id: control_id(id),
        label: label.to_string(),
        rect,
        enabled,
        visible: true,
        kind: UiControlKind::Button { on_activate },
    }
}

/// Close-menu button.
fn close_button(controller: &Rc<RefCell<NativeUiController>>, id: &str, label: &str, rect: Rect) -> UiControl {
    let controller = Rc::clone(controller);
    action_button(
        id,
        label,
        rect,
        true,
        Rc::new(move |_| {
            with_controller(&controller, |menu| menu.close_menu());
        }),
    )
}

/// Conflict menu for one pending replacement.
fn build_conflict_menu(
    conflict: &UiMenuId,
    controller: &Rc<RefCell<NativeUiController>>,
    input: &Rc<RefCell<dyn BindingStore>>,
    state: &Rc<RefCell<BindingMenuState>>,
    source: &Rc<dyn Fn() -> Vec<BindingAction>>,
) -> UiMenu {
    let request = match borrow_state(state, |menu| menu.pending.clone()) {
        Some(request) => request,
        None => panic!("No pending binding conflict"),
    };
    let actions = available_actions(source, input);
    let rows = MenuRowOptions::default();
    let replace_row = request.row.clone();
    let replace_physical = request.physical.clone();
    let input = Rc::clone(input);
    let state = Rc::clone(state);
    let controller = Rc::clone(controller);
    let close_state = Rc::clone(&state);
    UiMenu {
        scroll: None,
        id: conflict.clone(),
        title: format!("{} is already bound", physical_input_label(&request.physical)),
        full_screen: false,
        controls: vec![
            label_button(
                "ui:bindings:old",
                format!("Currently: {}", target_label(&request.previous, &actions)),
                menu_row(1, &rows),
            ),
            label_button(
                "ui:bindings:new",
                format!("Replace with: {}", request.row.action.label),
                menu_row(2, &rows),
            ),
            close_button(&controller, "ui:bindings:cancel", "Cancel", menu_row(4, &rows)),
            action_button(
                "ui:bindings:replace",
                "Replace binding",
                menu_row(5, &rows),
                true,
                Rc::new(move |_| {
                    assign_binding(&input, &state, &replace_row, &replace_physical);
                    with_controller(&controller, |menu| menu.close_menu());
                }),
            ),
        ],
        on_open: Rc::new(|_| {}),
        on_close: Rc::new(move |_| {
            borrow_state_mut(&close_state, |menu| {
                menu.pending = None;
            });
        }),
    }
}

/// Reset confirmation menu.
fn build_reset_menu(
    confirmation: &UiMenuId,
    controller: &Rc<RefCell<NativeUiController>>,
    state: &Rc<RefCell<BindingMenuState>>,
    reset: &BindingResetHandle,
) -> UiMenu {
    let rows = MenuRowOptions::default();
    let state = Rc::clone(state);
    let controller = Rc::clone(controller);
    let reset = reset.clone();
    UiMenu {
        scroll: None,
        id: confirmation.clone(),
        title: "Restore this player's default bindings?".to_string(),
        full_screen: false,
        controls: vec![
            close_button(&controller, "ui:bindings:keep", "Keep bindings", menu_row(3, &rows)),
            action_button(
                "ui:bindings:restore",
                "Restore defaults",
                menu_row(4, &rows),
                (reset.available)(),
                Rc::new(move |_| {
                    (reset.reset)();
                    borrow_state_mut(&state, |menu| {
                        menu.selected = None;
                    });
                    with_controller(&controller, |menu| menu.close_menu());
                }),
            ),
        ],
        on_open: Rc::new(|_| {}),
        on_close: Rc::new(|_| {}),
    }
}

/// Root binding list menu.
#[allow(clippy::too_many_arguments)]
fn build_root_menu(
    root: &UiMenuId,
    conflict: &UiMenuId,
    confirmation: Option<&UiMenuId>,
    controller: &Rc<RefCell<NativeUiController>>,
    input: &Rc<RefCell<dyn BindingStore>>,
    state: &Rc<RefCell<BindingMenuState>>,
    source: &Rc<dyn Fn() -> Vec<BindingAction>>,
    reset: Option<&BindingResetHandle>,
) -> UiMenu {
    let query = borrow_state(state, |menu| menu.query.clone());
    let actions = available_actions(source, input);
    let bindings = borrow_input(input, |store| store.bindings());
    let rows = binding_rows(&actions, &bindings, &query);
    let selected = borrow_state(state, |menu| menu.selected.clone());
    let current = rows
        .iter()
        .find(|row| Some(&row.id) == selected.as_ref())
        .or(rows.first())
        .cloned();
    let selected = current.as_ref().map(|row| row.id.clone());
    borrow_state_mut(state, |menu| {
        menu.selected = selected.clone();
    });
    let state_search = Rc::clone(state);
    let state_select = Rc::clone(state);
    let rows_activate = rows.clone();
    let controller_activate = Rc::clone(controller);
    let input_activate = Rc::clone(input);
    let state_activate = Rc::clone(state);
    let conflict_activate = conflict.clone();
    let mut controls = vec![
        UiControl {
            id: control_id("ui:bindings:search"),
            label: "Search actions".to_string(),
            rect: Rect {
                x: 48.0,
                y: 80.0,
                width: 544.0,
                height: 28.0,
            },
            enabled: true,
            visible: true,
            kind: UiControlKind::TextEntry {
                masked: false,
                text: query,
                maximum_length: 80,
                on_change: Rc::new(move |_, text: &str| {
                    borrow_state_mut(&state_search, |menu| {
                        menu.query = text.to_string();
                        menu.selected = None;
                    });
                }),
                on_submit: Rc::new(|_, _| {}),
            },
        },
        label_button(
            "ui:bindings:head",
            "Action".to_string(),
            Rect {
                x: 48.0,
                y: 112.0,
                width: 316.0,
                height: 24.0,
            },
        ),
        label_button(
            "ui:bindings:key-head",
            "Key".to_string(),
            Rect {
                x: 364.0,
                y: 112.0,
                width: 228.0,
                height: 24.0,
            },
        ),
        UiControl {
            id: control_id("ui:bindings:list"),
            label: "Bindings".to_string(),
            rect: Rect {
                x: 48.0,
                y: 136.0,
                width: 544.0,
                height: 240.0,
            },
            enabled: !rows.is_empty(),
            visible: true,
            kind: UiControlKind::List {
                row_height: Some(24.0),
                column_widths: Some(vec![316.0, 208.0]),
                on_activate: Some(Rc::new(move |_, id: &str| {
                    if let Some(row) = rows_activate.iter().find(|row| row.id == id) {
                        capture_row(
                            &controller_activate,
                            &input_activate,
                            &state_activate,
                            &conflict_activate,
                            row,
                        );
                    }
                })),
                rows: if rows.is_empty() {
                    vec![UiListRow {
                        action: None,
                        id: "empty".to_string(),
                        cells: vec!["No matching actions".to_string()],
                        image: None,
                        enabled: false,
                    }]
                } else {
                    rows.iter()
                        .map(|row| {
                            let key = match row.physical.as_ref() {
                                Some(physical) => physical_input_label(physical),
                                None => "Unbound".to_string(),
                            };
                            let action = match row.physical.clone() {
                                Some(physical) => {
                                    let input = Rc::clone(input);
                                    Some(ListRowAction {
                                        label: "X".to_string(),
                                        on_activate: Rc::new(move |_| {
                                            borrow_input_mut(&input, |store| {
                                                store.unbind(&physical);
                                            });
                                        }),
                                    })
                                }
                                None => None,
                            };
                            UiListRow {
                                action,
                                id: row.id.clone(),
                                cells: vec![row.action.label.clone(), key],
                                image: None,
                                enabled: true,
                            }
                        })
                        .collect()
                },
                selected: selected.clone(),
                on_select: Rc::new(move |_, id: &str| {
                    borrow_state_mut(&state_select, |menu| {
                        menu.selected = Some(id.to_string());
                    });
                }),
            },
        },
    ];
    let add_current = current.clone().map(|row| BindingRow {
        physical: None,
        target: row.action.target.clone(),
        ..row
    });
    let controller_add = Rc::clone(controller);
    let input_add = Rc::clone(input);
    let state_add = Rc::clone(state);
    let conflict_add = conflict.clone();
    controls.push(action_button(
        "ui:bindings:add",
        "Add binding",
        Rect {
            x: 48.0,
            y: 384.0,
            width: 176.0,
            height: 28.0,
        },
        current.is_some(),
        Rc::new(move |_| {
            if let Some(row) = add_current.as_ref() {
                capture_row(&controller_add, &input_add, &state_add, &conflict_add, row);
            }
        }),
    ));
    let remove_current = current.clone();
    let input_remove = Rc::clone(input);
    controls.push(action_button(
        "ui:bindings:remove",
        "Remove key",
        Rect {
            x: 232.0,
            y: 384.0,
            width: 176.0,
            height: 28.0,
        },
        current.as_ref().is_some_and(|row| row.physical.is_some()),
        Rc::new(move |_| {
            if let Some(row) = remove_current.as_ref() {
                if let Some(physical) = row.physical.as_ref() {
                    borrow_input_mut(&input_remove, |store| {
                        store.unbind(physical);
                    });
                }
            }
        }),
    ));
    let clear_current = current.clone();
    let input_clear = Rc::clone(input);
    controls.push(action_button(
        "ui:bindings:clear",
        "Clear action",
        Rect {
            x: 416.0,
            y: 384.0,
            width: 176.0,
            height: 28.0,
        },
        current.as_ref().is_some_and(|row| row.physical.is_some()),
        Rc::new(move |_| {
            if let Some(row) = clear_current.as_ref() {
                borrow_input_mut(&input_clear, |store| {
                    for binding in store.bindings() {
                        if binding_matches_action(&binding.target, &row.action) {
                            store.unbind(&binding.input);
                        }
                    }
                });
            }
        }),
    ));
    if let (Some(confirmation), Some(reset)) = (confirmation, reset) {
        let confirmation = confirmation.clone();
        let controller = Rc::clone(controller);
        controls.push(action_button(
            "ui:bindings:reset",
            "Reset bindings",
            Rect {
                x: 416.0,
                y: 420.0,
                width: 176.0,
                height: 28.0,
            },
            (reset.available)(),
            Rc::new(move |_| {
                with_controller(&controller, |menu| {
                    let _ = menu.open_menu(&confirmation);
                });
            }),
        ));
    }
    controls.push(close_button(
        controller,
        "ui:bindings:back",
        "Back",
        Rect {
            x: 48.0,
            y: 420.0,
            width: 100.0,
            height: 28.0,
        },
    ));
    UiMenu {
        scroll: None,
        id: root.clone(),
        title: "Bindings".to_string(),
        full_screen: false,
        controls,
        on_open: Rc::new(|_| {}),
        on_close: Rc::new(|_| {}),
    }
}

/// Register binding menus for one seat.
///
/// Binding edits remain seat-owned and conflicts require explicit
/// replacement. Panics when the controller and input belong to different
/// seats.
pub fn register_binding_menus(
    controller: &Rc<RefCell<NativeUiController>>,
    input: Rc<RefCell<dyn BindingStore>>,
    source: BindingSource,
    reset: Option<BindingResetHandle>,
) -> BindingMenus {
    let controller_seat = match controller.try_borrow() {
        Ok(borrowed) => borrowed.seat(),
        Err(_) => panic!("binding menu controller is already borrowed"),
    };
    let input_seat = borrow_input(&input, |store| store.seat());
    if controller_seat != input_seat {
        panic!("Binding menu belongs to another input seat");
    }
    let root = menu_id("menu:bindings:0");
    let conflict = menu_id("menu:bindings:conflict");
    let confirmation = reset.as_ref().map(|_| menu_id("menu:bindings:reset"));
    let source: Rc<dyn Fn() -> Vec<BindingAction>> = match source {
        BindingSource::Fixed(rows) => Rc::new(move || rows.clone()),
        BindingSource::Live(read) => read,
    };
    let state = Rc::new(RefCell::new(BindingMenuState::default()));
    let mut menus = vec![conflict.clone()];
    let conflict_factory_state = Rc::clone(&state);
    let conflict_factory_input = Rc::clone(&input);
    let conflict_factory_controller = Rc::clone(controller);
    let conflict_factory_source = Rc::clone(&source);
    let conflict_id = conflict.clone();
    with_controller(controller, |menu| {
        menu.register(
            conflict_id.clone(),
            Rc::new(move || {
                build_conflict_menu(
                    &conflict_id,
                    &conflict_factory_controller,
                    &conflict_factory_input,
                    &conflict_factory_state,
                    &conflict_factory_source,
                )
            }),
        );
    });
    if let Some(confirmation) = confirmation.clone() {
        menus.push(confirmation.clone());
        let reset = match reset.clone() {
            Some(reset) => reset,
            None => panic!("binding reset handle is missing"),
        };
        let factory_state = Rc::clone(&state);
        let factory_controller = Rc::clone(controller);
        with_controller(controller, move |menu| {
            menu.register(
                confirmation.clone(),
                Rc::new(move || build_reset_menu(&confirmation, &factory_controller, &factory_state, &reset)),
            );
        });
    }
    menus.push(root.clone());
    let root_id = root.clone();
    let conflict_id = conflict.clone();
    let confirmation_id = confirmation.clone();
    let factory_controller = Rc::clone(controller);
    let factory_input = Rc::clone(&input);
    let factory_state = Rc::clone(&state);
    let factory_source = Rc::clone(&source);
    let factory_reset = reset.clone();
    with_controller(controller, move |menu| {
        menu.register(
            root_id.clone(),
            Rc::new(move || {
                build_root_menu(
                    &root_id,
                    &conflict_id,
                    confirmation_id.as_ref(),
                    &factory_controller,
                    &factory_input,
                    &factory_state,
                    &factory_source,
                    factory_reset.as_ref(),
                )
            }),
        );
    });
    BindingMenus {
        root,
        controller: Rc::clone(controller),
        menus,
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::IdentityOwner;

    use super::*;
    use crate::ui::common::controller::headless_options;
    use crate::ui::types::{SeatInputEvent, SeatInputEventKind};

    fn owner() -> IdentityOwner {
        IdentityOwner::create("bindings-test").unwrap()
    }

    fn harness(
        actions: Vec<BindingAction>,
    ) -> (Rc<RefCell<NativeUiController>>, Rc<RefCell<dyn BindingStore>>, SeatId) {
        let _ = actions;
        let owner = owner();
        let seat = owner.seat(0);
        let controller = Rc::new(RefCell::new(NativeUiController::new(headless_options(seat.clone()))));
        let store: Rc<RefCell<dyn BindingStore>> = Rc::new(RefCell::new(SeatBindingStore::new(seat.clone())));
        (controller, store, seat)
    }

    fn command_action(id: &str, label: &str, text: &str) -> BindingAction {
        BindingAction {
            id: id.to_string(),
            label: label.to_string(),
            target: InputBindingTarget::Command(text.to_string()),
            matches: None,
        }
    }

    fn key_event(seat: &SeatId, code: i32) -> SeatInputEvent {
        SeatInputEvent {
            seat: seat.clone(),
            time_ms: 0,
            kind: SeatInputEventKind::Key {
                code,
                down: true,
                repeat: false,
            },
        }
    }

    fn button(control: &UiControl) -> Rc<dyn Fn(SeatId)> {
        match &control.kind {
            UiControlKind::Button { on_activate } => Rc::clone(on_activate),
            _ => panic!("expected button {}", control.id.as_str()),
        }
    }

    fn find<'a>(menu: &'a UiMenu, id: &str) -> &'a UiControl {
        menu.controls
            .iter()
            .find(|control| control.id.as_str() == id)
            .unwrap_or_else(|| panic!("missing control {id}"))
    }

    #[test]
    fn key_names_cover_table_and_edges() {
        assert_eq!(physical_input_label(&PhysicalInput::Key(-1)), "<KEY NOT FOUND>");
        assert_eq!(physical_input_label(&PhysicalInput::Key(-2)), "<OUT OF RANGE>");
        assert_eq!(physical_input_label(&PhysicalInput::Key(272)), "<OUT OF RANGE>");
        assert_eq!(physical_input_label(&PhysicalInput::Key(65)), "A");
        assert_eq!(physical_input_label(&PhysicalInput::Key(33)), "!");
        assert_eq!(physical_input_label(&PhysicalInput::Key(34)), "0x22");
        assert_eq!(physical_input_label(&PhysicalInput::Key(59)), "SEMICOLON");
        assert_eq!(physical_input_label(&PhysicalInput::Key(9)), "TAB");
        assert_eq!(physical_input_label(&PhysicalInput::Key(32)), "SPACE");
        assert_eq!(physical_input_label(&PhysicalInput::Key(145)), "F1");
        assert_eq!(physical_input_label(&PhysicalInput::Key(156)), "F12");
        assert_eq!(physical_input_label(&PhysicalInput::Key(178)), "MOUSE1");
        assert_eq!(physical_input_label(&PhysicalInput::Key(184)), "MWHEELUP");
        assert_eq!(physical_input_label(&PhysicalInput::Key(183)), "MWHEELDOWN");
        assert_eq!(physical_input_label(&PhysicalInput::Key(185)), "JOY1");
        assert_eq!(physical_input_label(&PhysicalInput::Key(216)), "JOY32");
        assert_eq!(physical_input_label(&PhysicalInput::Key(217)), "AUX1");
        assert_eq!(physical_input_label(&PhysicalInput::Key(232)), "AUX16");
        assert_eq!(physical_input_label(&PhysicalInput::Key(256)), "AUX17");
        assert_eq!(physical_input_label(&PhysicalInput::Key(271)), "AUX32");
        assert_eq!(physical_input_label(&PhysicalInput::Key(160)), "KP_HOME");
        assert_eq!(physical_input_label(&PhysicalInput::Key(131)), "PAUSE");
        assert_eq!(physical_input_label(&PhysicalInput::Key(130)), "0x82");
        assert_eq!(physical_input_label(&PhysicalInput::Key(0)), "0x00");
        assert_eq!(physical_input_label(&PhysicalInput::MouseButton(1)), "MOUSE1");
        assert_eq!(physical_input_label(&PhysicalInput::MouseButton(2)), "MOUSE3");
        assert_eq!(physical_input_label(&PhysicalInput::MouseButton(3)), "MOUSE2");
        assert_eq!(
            physical_input_label(&PhysicalInput::ControllerButton { device: 0, button: 0 }),
            "Pad 1 A"
        );
        assert_eq!(
            physical_input_label(&PhysicalInput::ControllerButton { device: 1, button: 7 }),
            "Pad 2 Left stick press"
        );
        assert_eq!(
            physical_input_label(&PhysicalInput::ControllerButton { device: 0, button: 99 }),
            "Pad 1 Button 100"
        );
        assert_eq!(
            physical_input_label(&PhysicalInput::ControllerAxis {
                device: 0,
                axis: ControllerAxis::LeftX,
                direction: AxisDirection::Positive,
            }),
            "Pad 1 Left stick right"
        );
        assert_eq!(
            physical_input_label(&PhysicalInput::ControllerAxis {
                device: 0,
                axis: ControllerAxis::LeftY,
                direction: AxisDirection::Negative,
            }),
            "Pad 1 Left stick up"
        );
        assert_eq!(
            physical_input_label(&PhysicalInput::ControllerAxis {
                device: 0,
                axis: ControllerAxis::RightTrigger,
                direction: AxisDirection::Negative,
            }),
            "Pad 1 Right trigger released"
        );
    }

    #[test]
    fn matching_uses_targets_and_custom_matchers() {
        let action = command_action("attack", "Fire", "+attack");
        assert!(binding_matches_action(
            &InputBindingTarget::Command("+attack".to_string()),
            &action
        ));
        assert!(!binding_matches_action(
            &InputBindingTarget::Command("+use".to_string()),
            &action
        ));
        let named = BindingAction {
            id: "jump".to_string(),
            label: "Jump".to_string(),
            target: InputBindingTarget::Action(InputAction::Jump),
            matches: None,
        };
        assert!(binding_matches_action(
            &InputBindingTarget::Action(InputAction::Jump),
            &named
        ));
        assert!(!binding_matches_action(
            &InputBindingTarget::Command("+jump".to_string()),
            &named
        ));
        let wheel = BindingAction {
            id: "wheel".to_string(),
            label: "Wheel".to_string(),
            target: InputBindingTarget::Command("+weaponwheel".to_string()),
            matches: Some(Rc::new(|target| {
                matches!(
                    target,
                    InputBindingTarget::Command(text) if text == "+wheel"
                )
            })),
        };
        assert!(binding_matches_action(
            &InputBindingTarget::Command("+wheel".to_string()),
            &wheel
        ));
    }

    #[test]
    fn seat_store_binds_and_unbinds() {
        let owner = owner();
        let seat = owner.seat(0);
        let mut store = SeatBindingStore::new(seat.clone());
        assert_eq!(store.seat(), seat);
        assert!(store.bindings().is_empty());
        let input = PhysicalInput::Key(87);
        store.bind(InputBinding {
            input: input.clone(),
            target: InputBindingTarget::Command("+forward".to_string()),
        });
        assert_eq!(
            store.binding(&input),
            Some(InputBindingTarget::Command("+forward".to_string()))
        );
        store.unbind(&input);
        assert_eq!(store.binding(&input), None);
    }

    #[test]
    fn custom_actions_synthesize_for_unknown_targets() {
        let (controller, input, _) = harness(vec![]);
        borrow_input_mut(&input, |store| {
            store.bind(InputBinding {
                input: PhysicalInput::Key(81),
                target: InputBindingTarget::Command("say hi".to_string()),
            });
            store.bind(InputBinding {
                input: PhysicalInput::Key(69),
                target: InputBindingTarget::Action(InputAction::Jump),
            });
        });
        let source: Rc<dyn Fn() -> Vec<BindingAction>> = Rc::new(Vec::new);
        let actions = available_actions(&source, &input);
        assert_eq!(actions.len(), 2);
        for action in &actions {
            assert!(action.id.starts_with("custom:"));
        }
        let labels: Vec<&str> = actions.iter().map(|action| action.label.as_str()).collect();
        assert!(labels.contains(&"Command: say hi"));
        assert!(labels.contains(&"Action: jump"));
        let _ = controller;
    }

    #[test]
    fn search_filters_rows() {
        let actions = vec![
            command_action("forward", "Move forward", "+forward"),
            command_action("attack", "Fire primary weapon", "+attack"),
        ];
        let bindings = vec![InputBinding {
            input: PhysicalInput::Key(87),
            target: InputBindingTarget::Command("+forward".to_string()),
        }];
        let rows = binding_rows(&actions, &bindings, "");
        assert_eq!(rows.len(), 2);
        let rows = binding_rows(&actions, &bindings, "fire");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].action.id, "attack");
        let rows = binding_rows(&actions, &bindings, "move W");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].action.id, "forward");
        let rows = binding_rows(&actions, &bindings, "nothing matches this");
        assert!(rows.is_empty());
    }

    #[test]
    fn root_menu_uses_donor_rects() {
        let (controller, input, _) = harness(vec![]);
        let menus = register_binding_menus(
            &controller,
            input,
            BindingSource::Fixed(vec![command_action("forward", "Move forward", "+forward")]),
            None,
        );
        assert_eq!(menus.root.as_str(), "menu:bindings:0");
        controller.borrow_mut().open_menu(&menus.root).unwrap();
        assert_eq!(controller.borrow().active_menu(), Some(menus.root.clone()));
        menus.dispose();
    }

    #[test]
    fn conflict_and_replace_flow() {
        let (controller, input, seat) = harness(vec![]);
        borrow_input_mut(&input, |store| {
            store.bind(InputBinding {
                input: PhysicalInput::Key(87),
                target: InputBindingTarget::Command("+attack".to_string()),
            });
        });
        let source: Rc<dyn Fn() -> Vec<BindingAction>> = Rc::new(|| {
            vec![
                command_action("forward", "Move forward", "+forward"),
                command_action("attack", "Fire primary weapon", "+attack"),
            ]
        });
        let state = Rc::new(RefCell::new(BindingMenuState::default()));
        let conflict = menu_id("menu:bindings:conflict");
        let factory_state = Rc::clone(&state);
        let factory_input = Rc::clone(&input);
        let factory_controller = Rc::clone(&controller);
        let factory_source = Rc::clone(&source);
        let conflict_id = conflict.clone();
        controller.borrow_mut().register(
            conflict.clone(),
            Rc::new(move || {
                build_conflict_menu(
                    &conflict_id,
                    &factory_controller,
                    &factory_input,
                    &factory_state,
                    &factory_source,
                )
            }),
        );
        let row = BindingRow {
            id: "forward".to_string(),
            action: command_action("forward", "Move forward", "+forward"),
            physical: None,
            target: InputBindingTarget::Command("+forward".to_string()),
        };
        capture_row(&controller, &input, &state, &conflict, &row);
        assert!(controller.borrow().binding_capture());
        controller.borrow_mut().input(&key_event(&seat, 87)).unwrap();
        assert_eq!(
            controller.borrow().active_menu().as_ref().map(|id| id.as_str()),
            Some("menu:bindings:conflict")
        );
        let pending = borrow_state(&state, |menu| menu.pending.clone()).unwrap();
        assert_eq!(pending.physical, PhysicalInput::Key(87));
        let menu = build_conflict_menu(&conflict, &controller, &input, &state, &source);
        assert_eq!(menu.title, "W is already bound");
        button(find(&menu, "ui:bindings:replace"))(seat.clone());
        assert_eq!(
            borrow_input(&input, |store| store.binding(&PhysicalInput::Key(87))),
            Some(InputBindingTarget::Command("+forward".to_string()))
        );
    }

    #[test]
    fn add_remove_and_clear() {
        let (controller, input, seat) = harness(vec![]);
        let menus = register_binding_menus(
            &controller,
            Rc::clone(&input),
            BindingSource::Fixed(vec![command_action("forward", "Move forward", "+forward")]),
            None,
        );
        controller.borrow_mut().open_menu(&menus.root).unwrap();
        let source: Rc<dyn Fn() -> Vec<BindingAction>> =
            Rc::new(|| vec![command_action("forward", "Move forward", "+forward")]);
        let state = Rc::new(RefCell::new(BindingMenuState::default()));
        let root = menu_id("menu:bindings:0");
        let conflict = menu_id("menu:bindings:conflict");
        let menu = build_root_menu(&root, &conflict, None, &controller, &input, &state, &source, None);
        let list = find(&menu, "ui:bindings:list");
        match &list.kind {
            UiControlKind::List { rows, .. } => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].cells[1], "Unbound");
            }
            _ => panic!("expected list"),
        }
        button(find(&menu, "ui:bindings:add"))(seat.clone());
        assert!(controller.borrow().binding_capture());
        controller.borrow_mut().input(&key_event(&seat, 87)).unwrap();
        assert_eq!(
            borrow_input(&input, |store| store.binding(&PhysicalInput::Key(87))),
            Some(InputBindingTarget::Command("+forward".to_string()))
        );
        let menu = build_root_menu(&root, &conflict, None, &controller, &input, &state, &source, None);
        button(find(&menu, "ui:bindings:remove"))(seat.clone());
        assert_eq!(
            borrow_input(&input, |store| store.binding(&PhysicalInput::Key(87))),
            None
        );
        borrow_input_mut(&input, |store| {
            store.bind(InputBinding {
                input: PhysicalInput::Key(87),
                target: InputBindingTarget::Command("+forward".to_string()),
            });
            store.bind(InputBinding {
                input: PhysicalInput::Key(38),
                target: InputBindingTarget::Command("+forward".to_string()),
            });
        });
        let menu = build_root_menu(&root, &conflict, None, &controller, &input, &state, &source, None);
        button(find(&menu, "ui:bindings:clear"))(seat.clone());
        assert!(borrow_input(&input, |store| store.bindings()).is_empty());
        menus.dispose();
    }

    #[test]
    fn reset_confirm_restores_and_clears_selection() {
        let (controller, input, seat) = harness(vec![]);
        let restored = Rc::new(RefCell::new(false));
        let restored_capture = Rc::clone(&restored);
        let reset = BindingResetHandle {
            available: Rc::new(|| true),
            reset: Rc::new(move || {
                *restored_capture.borrow_mut() = true;
            }),
        };
        let menus = register_binding_menus(
            &controller,
            Rc::clone(&input),
            BindingSource::Fixed(vec![command_action("forward", "Move forward", "+forward")]),
            Some(reset),
        );
        controller.borrow_mut().open_menu(&menus.root).unwrap();
        let confirmation = menu_id("menu:bindings:reset");
        controller.borrow_mut().open_menu(&confirmation).unwrap();
        assert_eq!(controller.borrow().active_menu(), Some(confirmation.clone()));
        let state = Rc::new(RefCell::new(BindingMenuState {
            query: String::new(),
            selected: Some("forward".to_string()),
            pending: None,
        }));
        let reset = BindingResetHandle {
            available: Rc::new(|| true),
            reset: Rc::new(move || {
                *restored.borrow_mut() = true;
            }),
        };
        let menu = build_reset_menu(&confirmation, &controller, &state, &reset);
        let restore = find(&menu, "ui:bindings:restore");
        assert!(restore.enabled);
        button(restore)(seat.clone());
        assert!(borrow_state(&state, |menu| menu.selected.is_none()));
        menus.dispose();
    }

    #[test]
    #[should_panic(expected = "Binding menu belongs to another input seat")]
    fn seat_mismatch_panics() {
        let owner = owner();
        let seat = owner.seat(0);
        let other = owner.seat(1);
        let controller = Rc::new(RefCell::new(NativeUiController::new(headless_options(seat))));
        let input: Rc<RefCell<dyn BindingStore>> = Rc::new(RefCell::new(SeatBindingStore::new(other)));
        let _ = register_binding_menus(&controller, input, BindingSource::Fixed(vec![]), None);
    }

    #[test]
    #[should_panic(expected = "No pending binding conflict")]
    fn conflict_without_pending_panics() {
        let (controller, input, _) = harness(vec![]);
        let state = Rc::new(RefCell::new(BindingMenuState::default()));
        let source: Rc<dyn Fn() -> Vec<BindingAction>> = Rc::new(Vec::new);
        let conflict = menu_id("menu:bindings:conflict");
        let _ = build_conflict_menu(&conflict, &controller, &input, &state, &source);
    }
}
