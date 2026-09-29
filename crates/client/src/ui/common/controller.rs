//! Native seat UI controller: menu stack, focus, fields, lists, and draw.
//!
//! Donor provenance: `src/ui/common/controller.ts` in full
//! (`NativeUiController`; navigation, wrapping, menu stack and fields follow
//! Q3 `ui_qmenu.c`/`ui_field.c`). Every mutable cursor, field, list offset,
//! and held controller direction belongs to one seat. Menu factories are
//! re-read before each input event and draw so live settings and feeders
//! stay fresh; a factory that returns a different identity is a caller bug.

use std::collections::HashMap;
use std::rc::Rc;

use qa_core::identity::SeatId;
use qa_core::math::{vec2, Vec2, Vec4};

use crate::error::ClientError;
use crate::input::{AxisDirection, ControllerAxis, InputBinding, KeyCode, PhysicalInput};
use crate::text::draw2d::Rect;
use crate::ui::common::accessibility::accessible_colors;
use crate::ui::common::layout::{contains, fit_ui, intersect, transform_ui, ui_point, UiTransform};
use crate::ui::common::skin::{default_ui_skin, nine_slice, UiSkin};
use crate::ui::types::{
    CenterPrintState, LegacyUiScript, ResourceId, SeatInputEvent, SeatInputEventKind, SeatInputFocus, SeatUiController,
    SeatUiState, TextAlign, UiAppearance, UiControl, UiControlId, UiControlKind, UiDrawCommand, UiDrawContext, UiMenu,
    UiMenuId, UiNotification,
};

/// Factory rebuilding one menu's live controls on every input event and draw.
pub type UiMenuFactory = Rc<dyn Fn() -> UiMenu>;

/// Text-run measurement service: width of `text` rendered at `scale`.
type MeasureText = dyn Fn(&str, f32) -> f32;

/// Menu feedback sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UiSound {
    /// Menu opened.
    Open,
    /// Menu closed.
    Close,
    /// Focus moved.
    Move,
    /// Value changed.
    Change,
    /// Action rejected.
    Reject,
}

/// Seat-owned services backing [`NativeUiController`].
pub struct NativeUiOptions {
    /// Owning seat; input and draws for any other seat are rejected.
    pub seat: SeatId,
    /// Current skin.
    pub skin: Box<dyn Fn() -> UiSkin>,
    /// Current time in milliseconds.
    pub now: Box<dyn Fn() -> i64>,
    /// Current seat bindings.
    pub bindings: Box<dyn Fn() -> Vec<InputBinding>>,
    /// Report an input-focus change.
    pub focus: Box<dyn FnMut(SeatInputFocus, i64)>,
    /// Play one menu sound.
    pub sound: Box<dyn FnMut(UiSound, SeatId)>,
    /// Execute one legacy menu script.
    pub execute_script: Box<dyn FnMut(LegacyUiScript, SeatId)>,
    /// Localize one menu string.
    pub localize: Box<dyn Fn(&str) -> String>,
    /// Current menu appearance.
    pub appearance: Box<dyn Fn() -> UiAppearance>,
    /// Clipboard text, if any.
    pub clipboard: Box<dyn Fn() -> Option<String>>,
    /// Measure one text run at one scale.
    pub measure_text: Box<MeasureText>,
}

/// Headless options for tests and dedicated servers: default skin, frozen
/// clock, no bindings, silent sinks, identity localization, and a fixed
/// eight-units-per-glyph measure.
#[must_use]
pub fn headless_options(seat: SeatId) -> NativeUiOptions {
    let font = match ResourceId::new("resource:engine:font") {
        Ok(font) => font,
        Err(_) => unreachable!("headless font id carries the resource namespace"),
    };
    let skin = default_ui_skin(&font);
    NativeUiOptions {
        seat,
        skin: Box::new(move || skin.clone()),
        now: Box::new(|| 0),
        bindings: Box::new(Vec::new),
        focus: Box::new(|_, _| {}),
        sound: Box::new(|_, _| {}),
        execute_script: Box::new(|_, _| {}),
        localize: Box::new(|text: &str| text.to_string()),
        appearance: Box::new(UiAppearance::default),
        clipboard: Box::new(|| None),
        measure_text: Box::new(|text: &str, scale: f32| text.chars().count() as f32 * 8.0 * scale),
    }
}

/// One pushed menu: identity, focused control, and scroll offset.
#[derive(Debug, Clone)]
struct MenuCursor {
    id: UiMenuId,
    focus: Option<UiControlId>,
    scroll: f32,
}

/// One text-entry cursor: caret, viewport start, and overstrike mode.
#[derive(Debug, Clone, Copy)]
struct FieldCursor {
    cursor: usize,
    start: usize,
    overstrike: bool,
}

/// One held controller direction awaiting its next repeat.
#[derive(Debug, Clone, Copy)]
struct HeldDirection {
    code: i32,
    next: i64,
}

/// One pending binding capture: accept a physical input or cancel.
struct BindingCapture {
    accept: Box<dyn FnMut(PhysicalInput)>,
    cancel: Box<dyn FnMut()>,
}

/// One laid-out list: visible top row, page size, and scrollbar geometry.
#[derive(Debug, Clone, Copy)]
struct ListLayout {
    top: usize,
    page: usize,
    height: f32,
    maximum: usize,
    thumb: f32,
}

fn enabled(control: &UiControl) -> bool {
    control.enabled && control.visible
}

fn quantize(minimum: f32, maximum: f32, step: f32, value: f32) -> f32 {
    (minimum + ((value - minimum) / step).round() * step).clamp(minimum, maximum)
}

/// Native menu controller for one seat.
pub struct NativeUiController {
    options: NativeUiOptions,
    menus: HashMap<UiMenuId, UiMenuFactory>,
    stack: Vec<MenuCursor>,
    fields: HashMap<UiControlId, FieldCursor>,
    list_tops: HashMap<UiControlId, usize>,
    list_rows: HashMap<UiControlId, Vec<String>>,
    list_drag_offset: f32,
    menu_drag_offset: Option<f32>,
    held_axes: HashMap<String, HeldDirection>,
    cursor: Vec2,
    pointer_position: Option<Vec2>,
    transform: UiTransform,
    dragging: Option<UiControlId>,
    shift: bool,
    control: bool,
    capture: Option<BindingCapture>,
    notifications: Vec<UiNotification>,
    center_print: Option<CenterPrintState>,
    scores: bool,
}

impl NativeUiController {
    /// Build a controller over one seat's options.
    #[must_use]
    pub fn new(options: NativeUiOptions) -> Self {
        Self {
            options,
            menus: HashMap::new(),
            stack: Vec::new(),
            fields: HashMap::new(),
            list_tops: HashMap::new(),
            list_rows: HashMap::new(),
            list_drag_offset: 0.0,
            menu_drag_offset: None,
            held_axes: HashMap::new(),
            cursor: vec2(320.0, 240.0),
            pointer_position: None,
            transform: UiTransform {
                x: 0.0,
                y: 0.0,
                scale: 1.0,
            },
            dragging: None,
            shift: false,
            control: false,
            capture: None,
            notifications: Vec::new(),
            center_print: None,
            scores: false,
        }
    }

    /// Owning seat.
    #[must_use]
    pub fn seat(&self) -> SeatId {
        self.options.seat.clone()
    }

    /// Register one menu factory, replacing any previous factory for the id.
    pub fn register(&mut self, id: UiMenuId, factory: UiMenuFactory) {
        self.menus.insert(id, factory);
    }

    /// Unregister one menu, closing it and every menu above it first.
    pub fn unregister(&mut self, id: &UiMenuId) {
        if let Some(index) = self.stack.iter().position(|menu| &menu.id == id) {
            while self.stack.len() > index {
                self.close_menu();
            }
        }
        self.menus.remove(id);
    }

    /// Whether one menu id is registered.
    #[must_use]
    pub fn is_registered(&self, id: &UiMenuId) -> bool {
        self.menus.contains_key(id)
    }

    /// Top menu id, if any menu is open.
    #[must_use]
    pub fn active_menu(&self) -> Option<UiMenuId> {
        self.stack.last().map(|menu| menu.id.clone())
    }

    /// Whether a binding capture is pending.
    #[must_use]
    pub fn binding_capture(&self) -> bool {
        self.capture.is_some()
    }

    /// Capture the next physical input, cancelling any pending capture.
    pub fn capture_binding(&mut self, accept: Box<dyn FnMut(PhysicalInput)>, cancel: Box<dyn FnMut()>) {
        if let Some(capture) = self.capture.as_mut() {
            (capture.cancel)();
        }
        self.capture = Some(BindingCapture { accept, cancel });
    }

    /// Replace the seat presentation state shown alongside menus.
    pub fn presentation(
        &mut self,
        notifications: Vec<UiNotification>,
        center_print: Option<CenterPrintState>,
        show_scores: bool,
    ) {
        self.notifications = notifications;
        self.center_print = center_print;
        self.scores = show_scores;
    }

    /// Close every open menu.
    pub fn close_all(&mut self) {
        while !self.stack.is_empty() {
            self.close_menu();
        }
    }

    fn active(&mut self) -> Result<Option<(UiMenu, usize)>, ClientError> {
        let Some(index) = self.stack.len().checked_sub(1) else {
            return Ok(None);
        };
        let factory =
            self.menus.get(&self.stack[index].id).cloned().ok_or_else(|| {
                ClientError::BadUi(format!("Active menu is not registered: {}", self.stack[index].id))
            })?;
        let menu = factory();
        if menu.id != self.stack[index].id {
            return Err(ClientError::BadUi(
                "Menu factory returned a different identity".to_string(),
            ));
        }
        let focus_valid = self.stack[index].focus.as_ref().is_some_and(|focus| {
            menu.controls
                .iter()
                .any(|control| &control.id == focus && enabled(control))
        });
        if !focus_valid {
            self.stack[index].focus = menu
                .controls
                .iter()
                .find(|control| enabled(control))
                .map(|control| control.id.clone());
        }
        let Some(scroll) = menu.scroll.clone() else {
            return Ok(Some((menu, index)));
        };
        let maximum = (scroll.content_height - scroll.rect.height).max(0.0);
        self.stack[index].scroll = self.stack[index].scroll.clamp(0.0, maximum);
        let offset = self.stack[index].scroll;
        let mut adjusted = menu;
        for control in &mut adjusted.controls {
            if scroll.controls.contains(&control.id) {
                control.rect.y -= offset;
            }
        }
        Ok(Some((adjusted, index)))
    }

    fn focus_changed(&mut self) {
        let focus = match self.stack.last() {
            None => SeatInputFocus::Game,
            Some(top) => SeatInputFocus::Menu {
                menu: top.id.clone(),
                control: top.focus.clone(),
            },
        };
        let now = (self.options.now)();
        (self.options.focus)(focus, now);
    }

    fn move_focus(&mut self, direction: i32) -> Result<(), ClientError> {
        let Some((menu, index)) = self.active()? else {
            return Ok(());
        };
        let controls: Vec<UiControl> = menu
            .controls
            .iter()
            .filter(|control| enabled(control))
            .cloned()
            .collect();
        if controls.is_empty() {
            return Ok(());
        }
        let current = controls
            .iter()
            .position(|control| Some(&control.id) == self.stack[index].focus.as_ref());
        let position = current.map_or(-1, |position| position as i32);
        let next = &controls[(position + direction).rem_euclid(controls.len() as i32) as usize];
        if Some(&next.id) != self.stack[index].focus.as_ref() {
            let next = next.clone();
            self.stack[index].focus = Some(next.id.clone());
            self.reveal(&menu, index, &next);
            self.focus_changed();
            self.play(UiSound::Move);
        }
        Ok(())
    }

    fn reveal(&mut self, menu: &UiMenu, index: usize, control: &UiControl) {
        let Some(scroll) = menu.scroll.as_ref() else {
            return;
        };
        if !scroll.controls.contains(&control.id) {
            return;
        }
        if control.rect.y < scroll.rect.y {
            self.stack[index].scroll -= scroll.rect.y - control.rect.y;
        } else if control.rect.y + control.rect.height > scroll.rect.y + scroll.rect.height {
            self.stack[index].scroll += control.rect.y + control.rect.height - scroll.rect.y - scroll.rect.height;
        }
        let maximum = (scroll.content_height - scroll.rect.height).max(0.0);
        self.stack[index].scroll = self.stack[index].scroll.clamp(0.0, maximum);
    }

    fn hit(&self, menu: &UiMenu, control: &UiControl) -> bool {
        if !enabled(control) || !contains(&control.rect, self.cursor) {
            return false;
        }
        match menu.scroll.as_ref() {
            None => true,
            Some(scroll) => !scroll.controls.contains(&control.id) || contains(&scroll.rect, self.cursor),
        }
    }

    fn menu_thumb(menu: &UiMenu) -> f32 {
        match menu.scroll.as_ref() {
            None => 0.0,
            Some(scroll) => scroll
                .rect
                .height
                .min((scroll.rect.height * scroll.rect.height / scroll.content_height.max(1.0)).max(24.0)),
        }
    }

    fn menu_pointer(&mut self, menu: &UiMenu, index: usize) {
        let (Some(scroll), Some(offset)) = (menu.scroll.as_ref(), self.menu_drag_offset) else {
            return;
        };
        let travel = scroll.rect.height - Self::menu_thumb(menu);
        let maximum = (scroll.content_height - scroll.rect.height).max(0.0);
        self.stack[index].scroll = if travel <= 0.0 {
            0.0
        } else {
            ((self.cursor.y - scroll.rect.y - offset) / travel * maximum).clamp(0.0, maximum)
        };
    }

    fn play(&mut self, sound: UiSound) {
        let seat = self.options.seat.clone();
        (self.options.sound)(sound, seat);
    }

    fn change(&mut self, control: &UiControl, direction: i32) -> Result<(), ClientError> {
        if !enabled(control) {
            return Ok(());
        }
        let seat = self.options.seat.clone();
        match &control.kind {
            UiControlKind::Toggle { checked, on_change } => {
                on_change(seat, !checked);
            }
            UiControlKind::Slider {
                minimum,
                maximum,
                step,
                value,
                on_change,
                ..
            } => {
                if *step <= 0.0 || minimum > maximum {
                    return Err(ClientError::BadUi("Invalid menu slider range".to_string()));
                }
                on_change(
                    seat,
                    quantize(*minimum, *maximum, *step, value + step * direction as f32),
                );
            }
            UiControlKind::Choice {
                choices,
                selected,
                on_select,
            } => {
                let count = choices.len() as i32;
                if count > 0 {
                    let current = choices
                        .iter()
                        .position(|choice| Some(&choice.id) == selected.as_ref())
                        .map_or(-1, |position| position as i32);
                    let next = &choices[(current + direction).rem_euclid(count) as usize];
                    on_select(seat, &next.id);
                }
            }
            _ => return Ok(()),
        }
        self.play(UiSound::Change);
        Ok(())
    }

    fn activate(&mut self, control: &UiControl) -> Result<(), ClientError> {
        if !enabled(control) {
            self.play(UiSound::Reject);
            return Ok(());
        }
        let seat = self.options.seat.clone();
        match &control.kind {
            UiControlKind::Button { on_activate } => {
                on_activate(seat);
                self.play(UiSound::Change);
            }
            UiControlKind::Toggle { .. } | UiControlKind::Slider { .. } | UiControlKind::Choice { .. } => {
                self.change(control, 1)?;
            }
            UiControlKind::TextEntry { text, on_submit, .. } => {
                on_submit(seat, text);
            }
            UiControlKind::List {
                selected,
                on_activate,
                on_select,
                ..
            } => {
                if let Some(selected) = selected {
                    match on_activate {
                        Some(on_activate) => on_activate(seat, selected),
                        None => on_select(seat, selected),
                    }
                }
            }
            UiControlKind::OwnerDraw { on_key, .. } => {
                on_key(seat, KeyCode::Enter as i32, true);
            }
        }
        Ok(())
    }

    fn field(&mut self, id: &UiControlId, length: usize) -> FieldCursor {
        let field = self.fields.entry(id.clone()).or_insert(FieldCursor {
            cursor: length,
            start: 0,
            overstrike: false,
        });
        field.cursor = field.cursor.min(length);
        *field
    }

    fn set_field(&mut self, id: &UiControlId, field: FieldCursor) {
        self.fields.insert(id.clone(), field);
    }

    fn insert_text(&mut self, control: &UiControl, text: &str) {
        let UiControlKind::TextEntry {
            text: current,
            maximum_length,
            on_change,
            ..
        } = &control.kind
        else {
            return;
        };
        let mut characters: Vec<char> = current.chars().collect();
        let field = self.field(&control.id, characters.len());
        let incoming: Vec<char> = text
            .chars()
            .filter(|character| !(*character < '\u{20}' || *character == '\u{7f}'))
            .collect();
        let room = maximum_length
            .saturating_sub(characters.len())
            .saturating_add(if field.overstrike {
                characters.len().saturating_sub(field.cursor)
            } else {
                0
            });
        let accepted = &incoming[..incoming.len().min(room)];
        let removed = if field.overstrike { accepted.len() } else { 0 };
        let end = (field.cursor + removed).min(characters.len());
        characters.splice(field.cursor..end, accepted.iter().copied());
        let updated: String = characters.iter().collect();
        self.set_field(
            &control.id,
            FieldCursor {
                cursor: field.cursor + accepted.len(),
                ..field
            },
        );
        on_change(self.options.seat.clone(), &updated);
    }

    fn field_key(&mut self, control: &UiControl, key: i32) -> bool {
        let UiControlKind::TextEntry {
            text: current,
            on_change,
            ..
        } = &control.kind
        else {
            return false;
        };
        let mut characters: Vec<char> = current.chars().collect();
        let mut field = self.field(&control.id, characters.len());
        let seat = self.options.seat.clone();
        if key == KeyCode::Left as i32 {
            field.cursor = field.cursor.saturating_sub(1);
        } else if key == KeyCode::Right as i32 {
            field.cursor = (field.cursor + 1).min(characters.len());
        } else if key == KeyCode::Home as i32 || self.control && key == 97 {
            field.cursor = 0;
        } else if key == KeyCode::End as i32 || self.control && key == 101 {
            field.cursor = characters.len();
        } else if key == KeyCode::Insert as i32 {
            field.overstrike = !field.overstrike;
        } else if key == KeyCode::Backspace as i32 || self.control && key == 104 {
            if field.cursor > 0 {
                field.cursor -= 1;
                if field.cursor < characters.len() {
                    characters.remove(field.cursor);
                }
                let updated: String = characters.iter().collect();
                self.set_field(&control.id, field);
                on_change(seat, &updated);
            } else {
                self.set_field(&control.id, field);
            }
            return true;
        } else if key == KeyCode::Delete as i32 {
            if field.cursor < characters.len() {
                characters.remove(field.cursor);
            }
            let updated: String = characters.iter().collect();
            self.set_field(&control.id, field);
            on_change(seat, &updated);
            return true;
        } else if self.control && key == 117 {
            field.cursor = 0;
            self.set_field(&control.id, field);
            on_change(seat, "");
            return true;
        } else if self.control && key == 118 {
            self.set_field(&control.id, field);
            if let Some(paste) = (self.options.clipboard)() {
                self.insert_text(control, &paste);
            }
            return true;
        } else {
            return false;
        }
        self.set_field(&control.id, field);
        true
    }

    fn list_layout(&mut self, control: &UiControl) -> Option<ListLayout> {
        let UiControlKind::List {
            row_height,
            rows,
            selected,
            ..
        } = &control.kind
        else {
            return None;
        };
        let height = row_height.unwrap_or_else(|| (self.options.skin)().line_height).max(1.0);
        let page = ((control.rect.height / height).floor() as usize).max(1);
        let maximum = rows.len().saturating_sub(page);
        let ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
        let changed = self.list_rows.get(&control.id).is_none_or(|previous| previous != &ids);
        if changed {
            self.list_rows.insert(control.id.clone(), ids);
            let top = rows
                .iter()
                .position(|row| Some(&row.id) == selected.as_ref())
                .map_or(0, |index| index.saturating_sub(page.saturating_sub(1)));
            self.list_tops.insert(control.id.clone(), top);
        }
        let top = self.list_tops.get(&control.id).copied().unwrap_or(0).min(maximum);
        self.list_tops.insert(control.id.clone(), top);
        let thumb = control
            .rect
            .height
            .min((control.rect.height * page as f32 / rows.len().max(1) as f32).max(24.0));
        Some(ListLayout {
            top,
            page,
            height,
            maximum,
            thumb,
        })
    }

    fn list_pointer(&mut self, control: &UiControl) {
        let Some(layout) = self.list_layout(control) else {
            return;
        };
        let travel = control.rect.height - layout.thumb;
        let top = if travel <= 0.0 {
            0
        } else {
            ((self.cursor.y - control.rect.y - self.list_drag_offset) / travel * layout.maximum as f32)
                .round()
                .clamp(0.0, layout.maximum as f32) as usize
        };
        self.list_tops.insert(control.id.clone(), top);
    }

    fn list_key(&mut self, control: &UiControl, key: i32) -> bool {
        let UiControlKind::List {
            rows,
            selected,
            on_select,
            ..
        } = &control.kind
        else {
            return false;
        };
        if key == KeyCode::Delete as i32 {
            if let Some(row) = rows.iter().find(|row| Some(&row.id) == selected.as_ref()) {
                if row.enabled {
                    if let Some(action) = row.action.as_ref() {
                        (action.on_activate)(self.options.seat.clone());
                    }
                }
            }
            return true;
        }
        let enabled: Vec<(usize, String)> = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.enabled)
            .map(|(index, row)| (index, row.id.clone()))
            .collect();
        let current = enabled.iter().position(|(_, id)| Some(id) == selected.as_ref());
        let page = self.list_layout(control).map_or(1, |layout| layout.page);
        let mut index = current.map_or(-1, |position| position as i32);
        if key == KeyCode::Up as i32 {
            index -= 1;
        } else if key == KeyCode::Down as i32 {
            index += 1;
        } else if key == KeyCode::PageUp as i32 {
            index -= page as i32;
        } else if key == KeyCode::PageDown as i32 {
            index += page as i32;
        } else if key == KeyCode::Home as i32 {
            index = 0;
        } else if key == KeyCode::End as i32 {
            index = enabled.len() as i32 - 1;
        } else {
            return false;
        }
        if enabled.is_empty() {
            return true;
        }
        let (source, id) = enabled[index.clamp(0, enabled.len() as i32 - 1) as usize].clone();
        on_select(self.options.seat.clone(), &id);
        let top = self.list_tops.get(&control.id).copied().unwrap_or(0);
        let next = if source < top {
            source
        } else if source >= top + page {
            source.saturating_sub(page - 1)
        } else {
            top
        };
        self.list_tops.insert(control.id.clone(), next);
        true
    }

    fn key(&mut self, code: i32, down: bool) -> Result<bool, ClientError> {
        if code == KeyCode::Shift as i32 {
            self.shift = down;
            return Ok(true);
        }
        if code == KeyCode::Control as i32 {
            self.control = down;
            return Ok(true);
        }
        let Some((menu, index)) = self.active()? else {
            return Ok(false);
        };
        let focused = menu
            .controls
            .iter()
            .find(|control| Some(&control.id) == self.stack[index].focus.as_ref())
            .cloned();
        if let Some(control) = focused.as_ref() {
            if let UiControlKind::OwnerDraw { on_key, .. } = &control.kind {
                if on_key(self.options.seat.clone(), code, down) {
                    return Ok(true);
                }
            }
        }
        if !down {
            return Ok(true);
        }
        if code == KeyCode::Escape as i32 {
            self.close_menu();
            return Ok(true);
        }
        if let Some(control) = focused.as_ref() {
            self.reveal(&menu, index, control);
        }
        if let Some(control) = focused.as_ref() {
            if matches!(control.kind, UiControlKind::TextEntry { .. }) && self.field_key(control, code) {
                return Ok(true);
            }
            if matches!(control.kind, UiControlKind::List { .. }) && self.list_key(control, code) {
                return Ok(true);
            }
        }
        if code == KeyCode::Tab as i32 {
            self.move_focus(if self.shift { -1 } else { 1 })?;
        } else if code == KeyCode::Up as i32 || code == KeyCode::KeypadUp as i32 {
            self.move_focus(-1)?;
        } else if code == KeyCode::Down as i32 || code == KeyCode::KeypadDown as i32 {
            self.move_focus(1)?;
        } else if let Some(control) = focused.as_ref() {
            if code == KeyCode::Left as i32 || code == KeyCode::KeypadLeft as i32 {
                self.change(control, -1)?;
            } else if code == KeyCode::Right as i32 || code == KeyCode::KeypadRight as i32 {
                self.change(control, 1)?;
            } else if code == KeyCode::Enter as i32
                || code == KeyCode::KeypadEnter as i32
                || code == KeyCode::Space as i32 && !matches!(control.kind, UiControlKind::TextEntry { .. })
            {
                self.activate(control)?;
            }
        }
        Ok(true)
    }

    fn pointer(&mut self, position: Vec2) -> Result<(), ClientError> {
        self.pointer_position = Some(position);
        self.cursor = ui_point(position, &self.transform);
        let Some((menu, index)) = self.active()? else {
            return Ok(());
        };
        if self.menu_drag_offset.is_some() {
            self.menu_pointer(&menu, index);
            return Ok(());
        }
        if let Some(drag) = menu
            .controls
            .iter()
            .find(|control| Some(&control.id) == self.dragging.as_ref())
        {
            if matches!(drag.kind, UiControlKind::Slider { .. }) {
                let drag = drag.clone();
                self.slider_pointer(&drag);
                return Ok(());
            }
            if matches!(drag.kind, UiControlKind::List { .. }) {
                let drag = drag.clone();
                self.list_pointer(&drag);
                return Ok(());
            }
        }
        let hovered = menu
            .controls
            .iter()
            .rev()
            .find(|control| self.hit(&menu, control))
            .map(|control| control.id.clone());
        if let Some(hovered) = hovered {
            if Some(&hovered) != self.stack[index].focus.as_ref() {
                self.stack[index].focus = Some(hovered);
                self.focus_changed();
                self.play(UiSound::Move);
            }
        }
        Ok(())
    }

    fn slider_pointer(&mut self, control: &UiControl) {
        let UiControlKind::Slider {
            minimum,
            maximum,
            step,
            on_change,
            ..
        } = &control.kind
        else {
            return;
        };
        let step = step.max(f32::MIN_POSITIVE);
        let left = control.rect.x + control.rect.width * 0.6;
        let width = control.rect.width * 0.32;
        let value = minimum + (maximum - minimum) * (self.cursor.x - left) / width;
        on_change(self.options.seat.clone(), quantize(*minimum, *maximum, step, value));
    }

    fn capture_event(&mut self, event: &SeatInputEvent) -> bool {
        if self.capture.is_none() {
            return false;
        }
        if let SeatInputEventKind::Key { code, down: true, .. } = &event.kind {
            if *code == KeyCode::Escape as i32 {
                if let Some(mut capture) = self.capture.take() {
                    (capture.cancel)();
                }
                return true;
            }
        }
        let input = match &event.kind {
            SeatInputEventKind::Key {
                code,
                down: true,
                repeat: false,
            } => Some(PhysicalInput::Key(*code)),
            SeatInputEventKind::MouseButton { button, down: true } => Some(PhysicalInput::MouseButton(*button)),
            SeatInputEventKind::MouseWheel { delta } if delta.y != 0.0 => Some(PhysicalInput::Key(if delta.y > 0.0 {
                KeyCode::MouseWheelUp as i32
            } else {
                KeyCode::MouseWheelDown as i32
            })),
            SeatInputEventKind::ControllerButton {
                device,
                button,
                down: true,
            } => Some(PhysicalInput::ControllerButton {
                device: *device,
                button: *button,
            }),
            SeatInputEventKind::ControllerAxis { device, axis, value } if value.abs() > 0.65 => {
                Some(PhysicalInput::ControllerAxis {
                    device: *device,
                    axis: *axis,
                    direction: if *value < 0.0 {
                        AxisDirection::Negative
                    } else {
                        AxisDirection::Positive
                    },
                })
            }
            _ => None,
        };
        if let Some(input) = input {
            if let Some(mut capture) = self.capture.take() {
                (capture.accept)(input);
            }
        }
        true
    }
}

impl SeatUiController for NativeUiController {
    fn seat(&self) -> SeatId {
        self.options.seat.clone()
    }

    fn state(&self) -> SeatUiState {
        let focus = match self.stack.last() {
            None => SeatInputFocus::Game,
            Some(top) => SeatInputFocus::Menu {
                menu: top.id.clone(),
                control: top.focus.clone(),
            },
        };
        SeatUiState {
            seat: self.options.seat.clone(),
            focus,
            cursor: self.cursor,
            bindings: (self.options.bindings)(),
            notifications: self.notifications.clone(),
            center_print: self.center_print.clone(),
            show_scores: self.scores,
        }
    }

    fn input(&mut self, event: &SeatInputEvent) -> Result<bool, ClientError> {
        if event.seat != self.options.seat {
            return Err(ClientError::BadUi("UI input delivered to another seat".to_string()));
        }
        if let SeatInputEventKind::Focus { focused: false } = &event.kind {
            self.dragging = None;
            self.menu_drag_offset = None;
            self.held_axes.clear();
            self.shift = false;
            self.control = false;
            if let Some(mut capture) = self.capture.take() {
                (capture.cancel)();
            }
        }
        if self.capture_event(event) {
            return Ok(true);
        }
        let Some((menu, index)) = self.active()? else {
            return Ok(false);
        };
        match &event.kind {
            SeatInputEventKind::Key { code, down, .. } => self.key(*code, *down),
            SeatInputEventKind::Text { text } => {
                if let Some(control) = menu
                    .controls
                    .iter()
                    .find(|control| Some(&control.id) == self.stack[index].focus.as_ref())
                    .cloned()
                {
                    if matches!(control.kind, UiControlKind::TextEntry { .. }) {
                        self.reveal(&menu, index, &control);
                        self.insert_text(&control, text);
                    }
                }
                Ok(true)
            }
            SeatInputEventKind::MouseMotion { position, .. } => {
                self.pointer(*position)?;
                Ok(true)
            }
            SeatInputEventKind::MouseButton { button, down } => {
                self.mouse_button(&menu, index, *button, *down)?;
                Ok(true)
            }
            SeatInputEventKind::MouseWheel { delta } => {
                self.mouse_wheel(&menu, index, *delta)?;
                Ok(true)
            }
            SeatInputEventKind::ControllerButton { button, down, .. } => {
                let code = match button {
                    0 => Some(KeyCode::Enter as i32),
                    1 | 6 => Some(KeyCode::Escape as i32),
                    11 => Some(KeyCode::Up as i32),
                    12 => Some(KeyCode::Down as i32),
                    13 => Some(KeyCode::Left as i32),
                    14 => Some(KeyCode::Right as i32),
                    _ => None,
                };
                if let Some(code) = code {
                    self.key(code, *down)?;
                }
                Ok(true)
            }
            SeatInputEventKind::ControllerAxis { device, axis, value } => {
                self.controller_axis(*device, *axis, *value, event.time_ms)?;
                Ok(true)
            }
            SeatInputEventKind::Focus { .. } => Ok(true),
        }
    }

    fn open_menu(&mut self, id: &UiMenuId) -> Result<(), ClientError> {
        let factory = self
            .menus
            .get(id)
            .cloned()
            .ok_or_else(|| ClientError::BadUi(format!("Unknown menu: {id}")))?;
        if let Some(existing) = self.stack.iter().position(|menu| &menu.id == id) {
            while self.stack.len() > existing + 1 {
                self.close_menu();
            }
            self.focus_changed();
            return Ok(());
        }
        let menu = factory();
        self.stack.push(MenuCursor {
            id: id.clone(),
            focus: menu
                .controls
                .iter()
                .find(|control| enabled(control))
                .map(|control| control.id.clone()),
            scroll: 0.0,
        });
        self.dragging = None;
        self.menu_drag_offset = None;
        self.held_axes.clear();
        (menu.on_open)(self.options.seat.clone());
        self.focus_changed();
        self.play(UiSound::Open);
        Ok(())
    }

    fn close_menu(&mut self) {
        let Some(current) = self.stack.pop() else {
            return;
        };
        if let Some(mut capture) = self.capture.take() {
            (capture.cancel)();
        }
        self.dragging = None;
        self.menu_drag_offset = None;
        self.held_axes.clear();
        if let Some(factory) = self.menus.get(&current.id).cloned() {
            (factory().on_close)(self.options.seat.clone());
        }
        self.focus_changed();
        self.play(UiSound::Close);
    }

    fn execute_script(&mut self, script: LegacyUiScript) {
        let seat = self.options.seat.clone();
        (self.options.execute_script)(script, seat);
    }

    fn draw(&mut self, context: &UiDrawContext) -> Result<Vec<UiDrawCommand>, ClientError> {
        if context.binding.seat != self.options.seat {
            return Err(ClientError::BadUi("UI draw delivered to another seat".to_string()));
        }
        let appearance = (self.options.appearance)();
        self.transform = fit_ui(&context.binding.safe_area, appearance.menu_scale)?;
        if let Some(position) = self.pointer_position {
            self.cursor = ui_point(position, &self.transform);
        }
        let due: Vec<String> = self
            .held_axes
            .iter()
            .filter(|(_, held)| context.time_ms >= held.next)
            .map(|(id, _)| id.clone())
            .collect();
        for id in due {
            if let Some(held) = self.held_axes.get(&id).copied() {
                self.key(held.code, true)?;
                if let Some(held) = self.held_axes.get_mut(&id) {
                    held.next = context.time_ms + 80;
                }
            }
        }
        let Some((menu, _)) = self.active()? else {
            return Ok(Vec::new());
        };
        let mut skin = (self.options.skin)();
        skin.font_scale *= appearance.text_scale;
        skin.colors = accessible_colors(&skin.colors, &appearance);
        let white = Vec4 {
            x: 1.0,
            y: 1.0,
            z: 1.0,
            w: 1.0,
        };
        let mut commands: Vec<UiDrawCommand> = Vec::new();
        if menu.full_screen {
            if let Some(background) = skin.background.as_ref() {
                commands.push(UiDrawCommand::Image {
                    rect: Rect {
                        x: 0.0,
                        y: 0.0,
                        width: 640.0,
                        height: 480.0,
                    },
                    resource: background.clone(),
                    tex_coords: [vec2(0.0, 0.0), vec2(1.0, 1.0)],
                    color: white,
                });
            }
        }
        let panel = Rect {
            x: 32.0,
            y: 24.0,
            width: 576.0,
            height: 432.0,
        };
        commands.push(UiDrawCommand::Fill {
            rect: panel,
            color: skin.colors.panel,
        });
        if let Some(slice) = skin.panel.as_ref() {
            commands.extend(nine_slice(slice, &panel, white)?);
        }
        match skin.title_font.as_ref() {
            None => self.emit_text(
                &mut commands,
                &skin,
                &menu.title,
                320.0,
                48.0,
                skin.colors.accent,
                TextAlign::Center,
                skin.font_scale,
            ),
            Some(title_font) => {
                let title = (self.options.localize)(&menu.title);
                let font = title_font.clone();
                let scale = skin.title_scale.unwrap_or(skin.font_scale);
                commands.push(UiDrawCommand::Text {
                    origin: vec2(64.0, 44.0),
                    text: title,
                    font,
                    scale,
                    color: skin.colors.accent,
                    align: TextAlign::Left,
                    shadow: true,
                });
            }
        }
        let controls = menu.controls.clone();
        for control in &controls {
            self.draw_control(context, &menu, &skin, white, &mut commands, control)?;
        }
        commands.push(UiDrawCommand::Clip { rect: None });
        if let Some(region) = menu.scroll.as_ref() {
            if region.content_height > region.rect.height {
                let thumb = Self::menu_thumb(&menu);
                let x = region.rect.x + region.rect.width - 14.0;
                commands.push(UiDrawCommand::Fill {
                    rect: Rect {
                        x,
                        y: region.rect.y,
                        width: 12.0,
                        height: region.rect.height,
                    },
                    color: skin.colors.control,
                });
                let scroll = self.stack.last().map_or(0.0, |cursor| cursor.scroll);
                commands.push(UiDrawCommand::Fill {
                    rect: Rect {
                        x,
                        y: region.rect.y
                            + (region.rect.height - thumb) * scroll / (region.content_height - region.rect.height),
                        width: 12.0,
                        height: thumb,
                    },
                    color: skin.colors.accent,
                });
            }
        }
        if self.capture.is_some() {
            let accent = skin.colors.accent;
            let scale = skin.font_scale;
            self.emit_text(
                &mut commands,
                &skin,
                "Press key/button. Esc cancels.",
                380.0,
                432.0,
                accent,
                TextAlign::Center,
                scale,
            );
        }
        let mut result = vec![UiDrawCommand::Clip {
            rect: Some(context.binding.safe_area),
        }];
        for command in &commands {
            let transformed = transform_ui(command, &self.transform);
            match transformed {
                UiDrawCommand::Clip { rect: None } => {
                    result.push(UiDrawCommand::Clip {
                        rect: Some(context.binding.safe_area),
                    });
                }
                UiDrawCommand::Clip { rect: Some(rect) } => {
                    result.push(UiDrawCommand::Clip {
                        rect: Some(intersect(&context.binding.safe_area, &rect)),
                    });
                }
                other => result.push(other),
            }
        }
        result.push(UiDrawCommand::Clip { rect: None });
        Ok(result)
    }
}

impl NativeUiController {
    fn mouse_button(&mut self, menu: &UiMenu, index: usize, button: i32, down: bool) -> Result<(), ClientError> {
        if !down {
            self.dragging = None;
            self.menu_drag_offset = None;
            return Ok(());
        }
        if button == 3 {
            self.close_menu();
            return Ok(());
        }
        if button != 1 {
            return Ok(());
        }
        if let Some(scroll) = menu.scroll.as_ref() {
            if scroll.content_height > scroll.rect.height
                && contains(&scroll.rect, self.cursor)
                && self.cursor.x >= scroll.rect.x + scroll.rect.width - 16.0
            {
                let thumb = Self::menu_thumb(menu);
                let y = scroll.rect.y
                    + (scroll.rect.height - thumb) * self.stack[index].scroll
                        / (scroll.content_height - scroll.rect.height);
                self.menu_drag_offset = Some(if self.cursor.y >= y && self.cursor.y < y + thumb {
                    self.cursor.y - y
                } else {
                    thumb / 2.0
                });
                self.menu_pointer(menu, index);
                return Ok(());
            }
        }
        let Some(control) = menu
            .controls
            .iter()
            .rev()
            .find(|control| self.hit(menu, control))
            .cloned()
        else {
            return Ok(());
        };
        self.stack[index].focus = Some(control.id.clone());
        self.focus_changed();
        match &control.kind {
            UiControlKind::Slider { .. } => {
                self.dragging = Some(control.id.clone());
                self.slider_pointer(&control);
            }
            UiControlKind::TextEntry { text, masked, .. } => {
                let shown: Vec<char> = if *masked {
                    text.chars().map(|_| '*').collect()
                } else {
                    text.chars().collect()
                };
                let x = self.cursor.x - control.rect.x - control.rect.width * 0.5;
                let scale = (self.options.skin)().font_scale * (self.options.appearance)().text_scale;
                let field = self.field(&control.id, shown.len());
                let mut cursor = field.start.min(shown.len());
                while cursor < shown.len() {
                    let run: String = shown[field.start.min(shown.len())..=cursor].iter().collect();
                    if (self.options.measure_text)(&run, scale) >= x {
                        break;
                    }
                    cursor += 1;
                }
                self.set_field(&control.id, FieldCursor { cursor, ..field });
            }
            UiControlKind::List { rows, .. } => {
                let Some(layout) = self.list_layout(&control) else {
                    return Ok(());
                };
                if layout.maximum > 0 && self.cursor.x >= control.rect.x + control.rect.width - 16.0 {
                    let thumb_y = control.rect.y
                        + (control.rect.height - layout.thumb) * layout.top as f32 / layout.maximum as f32;
                    self.list_drag_offset = if self.cursor.y >= thumb_y && self.cursor.y < thumb_y + layout.thumb {
                        self.cursor.y - thumb_y
                    } else {
                        layout.thumb / 2.0
                    };
                    self.dragging = Some(control.id.clone());
                    self.list_pointer(&control);
                } else {
                    let row_index =
                        layout.top + ((self.cursor.y - control.rect.y) / layout.height).floor().max(0.0) as usize;
                    if let Some(row) = rows.get(row_index) {
                        if row.enabled {
                            let seat = self.options.seat.clone();
                            let UiControlKind::List {
                                on_select, on_activate, ..
                            } = &control.kind
                            else {
                                return Ok(());
                            };
                            on_select(seat.clone(), &row.id);
                            let gutter = if layout.maximum > 0 { 16.0 } else { 0.0 };
                            if row.action.is_some()
                                && self.cursor.x >= control.rect.x + control.rect.width - gutter - 28.0
                            {
                                if let Some(action) = row.action.as_ref() {
                                    (action.on_activate)(seat);
                                }
                            } else if let Some(on_activate) = on_activate {
                                on_activate(seat, &row.id);
                            }
                        }
                    }
                }
            }
            _ => {
                self.activate(&control)?;
            }
        }
        Ok(())
    }

    fn mouse_wheel(&mut self, menu: &UiMenu, index: usize, delta: Vec2) -> Result<(), ClientError> {
        if let Some(scroll) = menu.scroll.as_ref() {
            if contains(&scroll.rect, self.cursor) {
                let maximum = (scroll.content_height - scroll.rect.height).max(0.0);
                self.stack[index].scroll = (self.stack[index].scroll - delta.y.signum() * 84.0).clamp(0.0, maximum);
                return Ok(());
            }
        }
        let hovered = menu
            .controls
            .iter()
            .find(|control| {
                enabled(control)
                    && matches!(control.kind, UiControlKind::List { .. })
                    && contains(&control.rect, self.cursor)
            })
            .cloned();
        let focused = menu
            .controls
            .iter()
            .find(|control| Some(&control.id) == self.stack[index].focus.as_ref())
            .cloned();
        if let Some(control) = hovered.or(focused) {
            if matches!(control.kind, UiControlKind::List { .. }) {
                if let Some(layout) = self.list_layout(&control) {
                    let top = (layout.top as f32 - delta.y.signum() * 3.0).clamp(0.0, layout.maximum as f32) as usize;
                    self.list_tops.insert(control.id.clone(), top);
                }
                return Ok(());
            }
        }
        if delta.y != 0.0 {
            self.key(
                if delta.y > 0.0 {
                    KeyCode::Up as i32
                } else {
                    KeyCode::Down as i32
                },
                true,
            )?;
        }
        Ok(())
    }

    fn controller_axis(
        &mut self,
        device: i32,
        axis: ControllerAxis,
        value: f32,
        time_ms: i64,
    ) -> Result<bool, ClientError> {
        if !matches!(axis, ControllerAxis::LeftX | ControllerAxis::LeftY) {
            return Ok(true);
        }
        let id = format!("{device}:{axis:?}");
        if value.abs() < 0.35 {
            self.held_axes.remove(&id);
            return Ok(true);
        }
        if value.abs() < 0.6 {
            return Ok(true);
        }
        let code = match axis {
            ControllerAxis::LeftX => {
                if value < 0.0 {
                    KeyCode::Left as i32
                } else {
                    KeyCode::Right as i32
                }
            }
            _ => {
                if value < 0.0 {
                    KeyCode::Up as i32
                } else {
                    KeyCode::Down as i32
                }
            }
        };
        if self.held_axes.get(&id).is_none_or(|held| held.code != code) {
            self.key(code, true)?;
            self.held_axes.insert(
                id,
                HeldDirection {
                    code,
                    next: time_ms + 300,
                },
            );
        }
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_text(
        &mut self,
        commands: &mut Vec<UiDrawCommand>,
        skin: &UiSkin,
        value: &str,
        x: f32,
        y: f32,
        color: Vec4,
        align: TextAlign,
        scale: f32,
    ) {
        let text = (self.options.localize)(value);
        commands.push(UiDrawCommand::Text {
            origin: vec2(x, y),
            text,
            font: skin.font.clone(),
            scale,
            color,
            align,
            shadow: true,
        });
    }

    fn measure(&mut self, value: &str, scale: f32) -> f32 {
        (self.options.measure_text)(value, scale)
    }

    fn draw_control(
        &mut self,
        context: &UiDrawContext,
        menu: &UiMenu,
        skin: &UiSkin,
        white: Vec4,
        commands: &mut Vec<UiDrawCommand>,
        control: &UiControl,
    ) -> Result<(), ClientError> {
        if !control.visible {
            return Ok(());
        }
        let scrolled = menu
            .scroll
            .as_ref()
            .is_some_and(|region| region.controls.contains(&control.id));
        if scrolled {
            if let Some(region) = menu.scroll.as_ref() {
                if control.rect.y + control.rect.height <= region.rect.y
                    || control.rect.y >= region.rect.y + region.rect.height
                {
                    return Ok(());
                }
                commands.push(UiDrawCommand::Clip {
                    rect: Some(region.rect),
                });
            }
        } else {
            commands.push(UiDrawCommand::Clip { rect: None });
        }
        let focused = Some(&control.id) == self.stack.last().and_then(|cursor| cursor.focus.as_ref());
        let color = if !control.enabled {
            skin.colors.disabled
        } else if focused {
            skin.colors.accent
        } else {
            skin.colors.text
        };
        if let UiControlKind::OwnerDraw { on_draw, .. } = &control.kind {
            let owned = UiDrawContext {
                binding: crate::ui::types::SeatPresentationBinding {
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
                    ..context.binding.clone()
                },
                time_ms: context.time_ms,
            };
            commands.extend(on_draw(&owned));
            return Ok(());
        }
        let focused_control = focused && !matches!(control.kind, UiControlKind::List { .. });
        let decoration = if focused_control {
            skin.focus.as_ref().or(skin.button.as_ref())
        } else {
            skin.button.as_ref()
        };
        commands.push(UiDrawCommand::Fill {
            rect: Rect {
                height: control.rect.height - 2.0,
                ..control.rect
            },
            color: if focused_control {
                skin.colors.focused
            } else {
                skin.colors.control
            },
        });
        if let Some(decoration) = decoration {
            commands.extend(nine_slice(decoration, &control.rect, white)?);
        }
        if matches!(control.kind, UiControlKind::List { .. }) {
            self.draw_list(skin, commands, control, focused)?;
            return Ok(());
        }
        if !matches!(control.kind, UiControlKind::Slider { .. }) {
            let label = control.label.clone();
            let scale = skin.font_scale;
            self.emit_text(
                commands,
                skin,
                &label,
                control.rect.x + 10.0,
                control.rect.y + 6.0,
                color,
                TextAlign::Left,
                scale,
            );
        }
        let right = control.rect.x + control.rect.width - 10.0;
        let y = control.rect.y + 6.0;
        match &control.kind {
            UiControlKind::Button { .. } | UiControlKind::OwnerDraw { .. } => {}
            UiControlKind::Toggle { checked, .. } => {
                let scale = skin.font_scale;
                self.emit_text(
                    commands,
                    skin,
                    if *checked { "On" } else { "Off" },
                    right,
                    y,
                    color,
                    TextAlign::Right,
                    scale,
                );
            }
            UiControlKind::Choice { choices, selected, .. } => {
                let label = choices
                    .iter()
                    .find(|choice| Some(&choice.id) == selected.as_ref())
                    .map_or("", |choice| choice.label.as_str())
                    .to_string();
                let scale = skin.font_scale;
                self.emit_text(commands, skin, &label, right, y, color, TextAlign::Right, scale);
            }
            UiControlKind::Slider {
                minimum,
                maximum,
                value,
                value_label,
                ..
            } => {
                self.draw_slider(
                    skin,
                    commands,
                    control,
                    color,
                    *minimum,
                    *maximum,
                    *value,
                    value_label.as_deref(),
                );
            }
            UiControlKind::TextEntry { text, masked, .. } => {
                let text = text.clone();
                let masked = *masked;
                let scale = skin.font_scale;
                self.draw_field(context, skin, commands, control, &text, masked, color, y, scale);
            }
            UiControlKind::List { .. } => {}
        }
        Ok(())
    }

    fn draw_list(
        &mut self,
        skin: &UiSkin,
        commands: &mut Vec<UiDrawCommand>,
        control: &UiControl,
        focused: bool,
    ) -> Result<(), ClientError> {
        let UiControlKind::List {
            rows,
            selected,
            column_widths,
            ..
        } = &control.kind
        else {
            return Ok(());
        };
        let Some(layout) = self.list_layout(control) else {
            return Ok(());
        };
        let content_width = control.rect.width - if layout.maximum > 0 { 16.0 } else { 0.0 };
        commands.push(UiDrawCommand::Clip {
            rect: Some(Rect {
                width: content_width,
                ..control.rect
            }),
        });
        let visible: Vec<(usize, crate::ui::types::UiListRow)> = rows
            .iter()
            .skip(layout.top)
            .take(layout.page)
            .cloned()
            .enumerate()
            .collect();
        for (index, row) in &visible {
            let y = control.rect.y + *index as f32 * layout.height;
            let selected_row = Some(&row.id) == selected.as_ref();
            if selected_row {
                commands.push(UiDrawCommand::Fill {
                    rect: Rect {
                        x: control.rect.x,
                        y,
                        width: content_width,
                        height: layout.height,
                    },
                    color: skin.colors.focused,
                });
            }
            let row_color = if !row.enabled {
                skin.colors.disabled
            } else if selected_row {
                skin.colors.accent
            } else {
                skin.colors.text
            };
            match column_widths {
                None => {
                    let joined = row.cells.join("  ");
                    let scale = skin.font_scale;
                    self.emit_text(
                        commands,
                        skin,
                        &joined,
                        control.rect.x + 8.0,
                        y,
                        row_color,
                        TextAlign::Left,
                        scale,
                    );
                }
                Some(widths) => {
                    let mut x = control.rect.x;
                    for (column, value) in row.cells.iter().enumerate() {
                        let reserve = if row.action.is_none() { 0.0 } else { 28.0 };
                        let width = widths
                            .get(column)
                            .copied()
                            .unwrap_or(content_width)
                            .min(control.rect.x + content_width - reserve - x);
                        let measured = self.measure(value, skin.font_scale);
                        let available = (width - 16.0).max(0.0);
                        let scale = skin.font_scale * (available / measured.max(1.0)).clamp(0.75, 1.0).min(1.0);
                        let mut label = value.clone();
                        if measured * scale / skin.font_scale > available {
                            label = self.ellipsize(value, scale, available);
                        }
                        let font = skin.font.clone();
                        commands.push(UiDrawCommand::Text {
                            origin: vec2(x + 8.0, y + (layout.height - 8.0 * scale) / 2.0),
                            text: label,
                            font,
                            scale,
                            color: row_color,
                            align: TextAlign::Left,
                            shadow: true,
                        });
                        x += width;
                    }
                }
            }
            if let Some(action) = row.action.as_ref() {
                let scale = skin.font_scale;
                self.emit_text(
                    commands,
                    skin,
                    &action.label.clone(),
                    control.rect.x + content_width - 14.0,
                    y + (layout.height - 8.0 * skin.font_scale) / 2.0,
                    row_color,
                    TextAlign::Center,
                    scale,
                );
            }
        }
        commands.push(UiDrawCommand::Clip { rect: None });
        if layout.maximum > 0 {
            let x = control.rect.x + control.rect.width - 14.0;
            commands.push(UiDrawCommand::Fill {
                rect: Rect {
                    x,
                    y: control.rect.y,
                    width: 12.0,
                    height: control.rect.height,
                },
                color: skin.colors.control,
            });
            commands.push(UiDrawCommand::Fill {
                rect: Rect {
                    x,
                    y: control.rect.y
                        + (control.rect.height - layout.thumb) * layout.top as f32 / layout.maximum as f32,
                    width: 12.0,
                    height: layout.thumb,
                },
                color: if focused {
                    skin.colors.accent
                } else {
                    skin.colors.disabled
                },
            });
        }
        Ok(())
    }

    fn ellipsize(&mut self, value: &str, scale: f32, available: f32) -> String {
        let glyphs: Vec<char> = value.chars().collect();
        let mut low = 0;
        let mut high = glyphs.len();
        while low < high {
            let middle = low + (high - low).div_ceil(2);
            let candidate: String = glyphs[..middle].iter().collect::<String>() + "\u{2026}";
            if self.measure(&candidate, scale) <= available {
                low = middle;
            } else {
                high = middle - 1;
            }
        }
        if self.measure("\u{2026}", scale) <= available {
            glyphs[..low].iter().collect::<String>() + "\u{2026}"
        } else {
            String::new()
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_slider(
        &mut self,
        skin: &UiSkin,
        commands: &mut Vec<UiDrawCommand>,
        control: &UiControl,
        color: Vec4,
        minimum: f32,
        maximum: f32,
        value: f32,
        value_label: Option<&str>,
    ) {
        let y = control.rect.y + 6.0;
        let x = control.rect.x + control.rect.width * 0.6;
        let width = control.rect.width * 0.32;
        let formatted = format!("{value:.6}").parse::<f64>().map(|number| number.to_string());
        let fallback = format!("{value:.6}");
        let value_text = value_label
            .map(str::to_string)
            .unwrap_or_else(|| formatted.unwrap_or(fallback));
        let value_right = x - 12.0;
        let value_scale = skin.font_scale
            * (control.rect.width * 0.16 / self.measure(&value_text, skin.font_scale).max(1.0)).min(1.0);
        let label_width = value_right
            - self.measure(&value_text, skin.font_scale) * value_scale / skin.font_scale
            - 12.0
            - (control.rect.x + 10.0);
        let label_scale =
            skin.font_scale * (label_width.max(1.0) / self.measure(&control.label, skin.font_scale).max(1.0)).min(1.0);
        let label = control.label.clone();
        self.emit_text(
            commands,
            skin,
            &label,
            control.rect.x + 10.0,
            y,
            color,
            TextAlign::Left,
            label_scale,
        );
        self.emit_text(
            commands,
            skin,
            &value_text,
            value_right,
            y,
            color,
            TextAlign::Right,
            value_scale,
        );
        let ratio = if maximum == minimum {
            0.0
        } else {
            ((value - minimum) / (maximum - minimum)).clamp(0.0, 1.0)
        };
        commands.push(UiDrawCommand::Fill {
            rect: Rect {
                x,
                y: y + 7.0,
                width,
                height: 2.0,
            },
            color: skin.colors.disabled,
        });
        commands.push(UiDrawCommand::Fill {
            rect: Rect {
                x: x + width * ratio - 3.0,
                y: y + 2.0,
                width: 6.0,
                height: 12.0,
            },
            color,
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_field(
        &mut self,
        context: &UiDrawContext,
        skin: &UiSkin,
        commands: &mut Vec<UiDrawCommand>,
        control: &UiControl,
        text: &str,
        masked: bool,
        color: Vec4,
        y: f32,
        scale: f32,
    ) {
        let all: Vec<char> = if masked {
            text.chars().map(|_| '*').collect()
        } else {
            text.chars().collect()
        };
        let available = control.rect.width * 0.5 - 12.0;
        let field = self.field(&control.id, all.len());
        let mut start = field.start.min(field.cursor);
        let mut end = field.cursor;
        while start < field.cursor {
            let run: String = all[start..field.cursor].iter().collect();
            if self.measure(&run, scale) <= available {
                break;
            }
            start += 1;
        }
        while end < all.len() {
            let run: String = all[start..=end].iter().collect();
            if self.measure(&run, scale) > available {
                break;
            }
            end += 1;
        }
        self.set_field(&control.id, FieldCursor { start, ..field });
        let shown: String = all[start..end].iter().collect();
        let x = control.rect.x + control.rect.width * 0.5;
        self.emit_text(commands, skin, &shown, x, y, color, TextAlign::Left, scale);
        let focused = Some(&control.id) == self.stack.last().and_then(|cursor| cursor.focus.as_ref());
        if focused && context.time_ms / 256 % 2 == 0 {
            let prefix: String = all[start..field.cursor.min(all.len())].iter().collect();
            let caret_x = x + self.measure(&prefix, scale);
            let caret = if field.overstrike { "_" } else { "|" };
            self.emit_text(commands, skin, caret, caret_x, y, color, TextAlign::Left, scale);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use qa_core::identity::IdentityOwner;
    use qa_core::math::{vec2, vec4};

    use super::*;
    use crate::ui::types::{
        ContentId, DopplerSelection, EnvironmentSelection, ListRowAction, PresentationSelection, ProviderRef,
        SeatPresentationBinding, UiChoice, UiListRow,
    };

    fn owner() -> IdentityOwner {
        IdentityOwner::create("controller-test").unwrap()
    }

    fn binding(owner: &IdentityOwner, seat: &SeatId) -> SeatPresentationBinding {
        let provider = |name: &str| ProviderRef {
            provider: name.to_string(),
            content: ContentId::new(name),
        };
        SeatPresentationBinding {
            seat: seat.clone(),
            client: owner.client(0, 0),
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
                doppler: DopplerSelection::Disabled,
                environment: EnvironmentSelection::Disabled,
                assets: ContentId::new("assets"),
                hud: provider("hud"),
                effects: provider("effects"),
                audio: provider("audio"),
            },
        }
    }

    fn context(owner: &IdentityOwner, seat: &SeatId, time_ms: i64) -> UiDrawContext {
        UiDrawContext {
            binding: binding(owner, seat),
            time_ms,
        }
    }

    fn button(id: &str, label: &str, y: f32, hits: Rc<RefCell<u32>>) -> UiControl {
        UiControl {
            id: UiControlId::new(id).unwrap(),
            label: label.to_string(),
            rect: Rect {
                x: 64.0,
                y,
                width: 512.0,
                height: 28.0,
            },
            enabled: true,
            visible: true,
            kind: UiControlKind::Button {
                on_activate: Rc::new(move |_| *hits.borrow_mut() += 1),
            },
        }
    }

    fn menu(id: &str, title: &str, controls: Vec<UiControl>) -> UiMenu {
        UiMenu {
            scroll: None,
            id: UiMenuId::new(id).unwrap(),
            title: title.to_string(),
            full_screen: false,
            controls,
            on_open: Rc::new(|_| {}),
            on_close: Rc::new(|_| {}),
        }
    }

    fn key(seat: &SeatId, code: i32, down: bool) -> SeatInputEvent {
        SeatInputEvent {
            seat: seat.clone(),
            time_ms: 0,
            kind: SeatInputEventKind::Key {
                code,
                down,
                repeat: false,
            },
        }
    }

    fn press(controller: &mut NativeUiController, seat: &SeatId, code: i32) {
        controller.input(&key(seat, code, true)).unwrap();
        controller.input(&key(seat, code, false)).unwrap();
    }

    fn focused(controller: &NativeUiController) -> Option<String> {
        match controller.state().focus {
            SeatInputFocus::Menu { control, .. } => control.map(|id| id.as_str().to_string()),
            _ => None,
        }
    }

    #[test]
    fn stack_open_close_and_reopen_focus() {
        let owner = owner();
        let seat = owner.seat(0);
        let mut controller = NativeUiController::new(headless_options(seat.clone()));
        let a = UiMenuId::new("menu:t:a").unwrap();
        let b = UiMenuId::new("menu:t:b").unwrap();
        controller.register(a.clone(), Rc::new(|| menu("menu:t:a", "A", vec![])));
        controller.register(b.clone(), Rc::new(|| menu("menu:t:b", "B", vec![])));
        assert!(controller.is_registered(&a));
        assert!(controller.open_menu(&UiMenuId::new("menu:t:missing").unwrap()).is_err());
        controller.open_menu(&a).unwrap();
        controller.open_menu(&b).unwrap();
        assert_eq!(controller.active_menu(), Some(b.clone()));
        controller.open_menu(&a).unwrap();
        assert_eq!(controller.active_menu(), Some(a.clone()));
        assert!(matches!(controller.state().focus, SeatInputFocus::Menu { .. }));
        controller.close_all();
        assert_eq!(controller.active_menu(), None);
        assert_eq!(controller.state().focus, SeatInputFocus::Game);
        controller.unregister(&a);
        assert!(!controller.is_registered(&a));
    }

    #[test]
    fn focus_wraps_at_both_ends() {
        let owner = owner();
        let seat = owner.seat(0);
        let mut controller = NativeUiController::new(headless_options(seat.clone()));
        let hits = Rc::new(RefCell::new(0));
        let first = button("ui:t:first", "First", 92.0, hits.clone());
        let second = button("ui:t:second", "Second", 120.0, hits);
        controller.register(
            UiMenuId::new("menu:t:wrap").unwrap(),
            Rc::new(move || menu("menu:t:wrap", "Wrap", vec![first.clone(), second.clone()])),
        );
        controller.open_menu(&UiMenuId::new("menu:t:wrap").unwrap()).unwrap();
        assert_eq!(focused(&controller).as_deref(), Some("ui:t:first"));
        press(&mut controller, &seat, KeyCode::Up as i32);
        assert_eq!(focused(&controller).as_deref(), Some("ui:t:second"));
        press(&mut controller, &seat, KeyCode::Down as i32);
        assert_eq!(focused(&controller).as_deref(), Some("ui:t:first"));
        press(&mut controller, &seat, KeyCode::Enter as i32);
    }

    fn controller_register(controller: &mut NativeUiController, slider: UiControl, seat: &SeatId) {
        controller.register(
            UiMenuId::new("menu:t:slider").unwrap(),
            Rc::new(move || menu("menu:t:slider", "S", vec![slider.clone()])),
        );
        controller.open_menu(&UiMenuId::new("menu:t:slider").unwrap()).unwrap();
        press(controller, seat, KeyCode::Right as i32);
        press(controller, seat, KeyCode::Left as i32);
        controller
            .input(&SeatInputEvent {
                seat: seat.clone(),
                time_ms: 0,
                kind: SeatInputEventKind::MouseMotion {
                    position: vec2(371.2, 100.0),
                    delta: vec2(0.0, 0.0),
                },
            })
            .unwrap();
        controller
            .input(&SeatInputEvent {
                seat: seat.clone(),
                time_ms: 0,
                kind: SeatInputEventKind::MouseButton { button: 1, down: true },
            })
            .unwrap();
        controller
            .input(&SeatInputEvent {
                seat: seat.clone(),
                time_ms: 0,
                kind: SeatInputEventKind::MouseMotion {
                    position: vec2(535.04, 100.0),
                    delta: vec2(0.0, 0.0),
                },
            })
            .unwrap();
    }

    #[test]
    fn slider_values_are_quantized() {
        let owner = owner();
        let seat = owner.seat(0);
        let values = Rc::new(RefCell::new(Vec::new()));
        let seen = values.clone();
        let slider = UiControl {
            id: UiControlId::new("ui:t:volume").unwrap(),
            label: "Volume".to_string(),
            rect: Rect {
                x: 64.0,
                y: 92.0,
                width: 512.0,
                height: 28.0,
            },
            enabled: true,
            visible: true,
            kind: UiControlKind::Slider {
                minimum: 0.0,
                maximum: 10.0,
                step: 2.0,
                value: 5.0,
                value_label: None,
                on_change: Rc::new(move |_, value| seen.borrow_mut().push(value)),
            },
        };
        let mut controller = NativeUiController::new(headless_options(seat.clone()));
        controller_register(&mut controller, slider, &seat);
        assert_eq!(*values.borrow(), vec![8.0, 4.0, 0.0, 10.0]);
        let bad = UiControl {
            id: UiControlId::new("ui:t:bad").unwrap(),
            label: "Bad".to_string(),
            rect: Rect {
                x: 64.0,
                y: 92.0,
                width: 512.0,
                height: 28.0,
            },
            enabled: true,
            visible: true,
            kind: UiControlKind::Slider {
                minimum: 0.0,
                maximum: 10.0,
                step: 0.0,
                value: 5.0,
                value_label: None,
                on_change: Rc::new(|_, _| {}),
            },
        };
        let mut broken = NativeUiController::new(headless_options(seat.clone()));
        broken.register(
            UiMenuId::new("menu:t:bad").unwrap(),
            Rc::new(move || menu("menu:t:bad", "B", vec![bad.clone()])),
        );
        broken.open_menu(&UiMenuId::new("menu:t:bad").unwrap()).unwrap();
        assert!(broken.input(&key(&seat, KeyCode::Right as i32, true)).is_err());
    }

    #[test]
    fn choice_wraps_past_both_ends() {
        let owner = owner();
        let seat = owner.seat(0);
        let selected = Rc::new(RefCell::new(Vec::new()));
        let seen = selected.clone();
        let live = Rc::new(RefCell::new(Some("b".to_string())));
        let current = live.clone();
        let stored = live.clone();
        let choice = move || {
            let seen = seen.clone();
            let stored = stored.clone();
            UiControl {
                id: UiControlId::new("ui:t:mode").unwrap(),
                label: "Mode".to_string(),
                rect: Rect {
                    x: 64.0,
                    y: 92.0,
                    width: 512.0,
                    height: 28.0,
                },
                enabled: true,
                visible: true,
                kind: UiControlKind::Choice {
                    choices: vec![
                        UiChoice {
                            id: "a".to_string(),
                            label: "A".to_string(),
                        },
                        UiChoice {
                            id: "b".to_string(),
                            label: "B".to_string(),
                        },
                    ],
                    selected: current.borrow().clone(),
                    on_select: Rc::new(move |_, id| {
                        seen.borrow_mut().push(id.to_string());
                        *stored.borrow_mut() = Some(id.to_string());
                    }),
                },
            }
        };
        let mut controller = NativeUiController::new(headless_options(seat.clone()));
        controller.register(
            UiMenuId::new("menu:t:choice").unwrap(),
            Rc::new(move || menu("menu:t:choice", "C", vec![choice()])),
        );
        controller.open_menu(&UiMenuId::new("menu:t:choice").unwrap()).unwrap();
        press(&mut controller, &seat, KeyCode::Right as i32);
        press(&mut controller, &seat, KeyCode::Left as i32);
        press(&mut controller, &seat, KeyCode::Enter as i32);
        assert_eq!(
            *selected.borrow(),
            vec!["a".to_string(), "b".to_string(), "a".to_string()]
        );
    }

    #[test]
    fn text_entry_inserts_filters_deletes_and_clicks() {
        let owner = owner();
        let seat = owner.seat(0);
        let live = Rc::new(RefCell::new(String::new()));
        let stored = live.clone();
        let written = live.clone();
        let field = move || {
            let written = written.clone();
            UiControl {
                id: UiControlId::new("ui:t:name").unwrap(),
                label: "Name".to_string(),
                rect: Rect {
                    x: 64.0,
                    y: 92.0,
                    width: 512.0,
                    height: 28.0,
                },
                enabled: true,
                visible: true,
                kind: UiControlKind::TextEntry {
                    masked: false,
                    text: stored.borrow().clone(),
                    maximum_length: 8,
                    on_change: Rc::new(move |_, text| *written.borrow_mut() = text.to_string()),
                    on_submit: Rc::new(|_, _| {}),
                },
            }
        };
        let mut controller = NativeUiController::new(headless_options(seat.clone()));
        controller.register(
            UiMenuId::new("menu:t:field").unwrap(),
            Rc::new(move || menu("menu:t:field", "F", vec![field()])),
        );
        controller.open_menu(&UiMenuId::new("menu:t:field").unwrap()).unwrap();
        controller
            .input(&SeatInputEvent {
                seat: seat.clone(),
                time_ms: 0,
                kind: SeatInputEventKind::Text {
                    text: "a\u{1}b\u{7f}c".to_string(),
                },
            })
            .unwrap();
        assert_eq!(*live.borrow(), "abc");
        press(&mut controller, &seat, KeyCode::Backspace as i32);
        assert_eq!(*live.borrow(), "ab");
        press(&mut controller, &seat, KeyCode::Delete as i32);
        assert_eq!(*live.borrow(), "ab");
        press(&mut controller, &seat, KeyCode::Left as i32);
        press(&mut controller, &seat, KeyCode::Delete as i32);
        assert_eq!(*live.borrow(), "a");
        controller
            .input(&SeatInputEvent {
                seat: seat.clone(),
                time_ms: 0,
                kind: SeatInputEventKind::MouseMotion {
                    position: vec2(320.0, 100.0),
                    delta: vec2(0.0, 0.0),
                },
            })
            .unwrap();
        controller
            .input(&SeatInputEvent {
                seat: seat.clone(),
                time_ms: 0,
                kind: SeatInputEventKind::MouseButton { button: 1, down: true },
            })
            .unwrap();
        press(&mut controller, &seat, KeyCode::Backspace as i32);
        assert_eq!(*live.borrow(), "a");
    }

    #[test]
    fn list_scrolls_selects_activates_and_runs_row_actions() {
        let owner = owner();
        let seat = owner.seat(0);
        let selected = Rc::new(RefCell::new(Vec::new()));
        let seen = selected.clone();
        let activated = Rc::new(RefCell::new(Vec::new()));
        let fired = activated.clone();
        let acted = Rc::new(RefCell::new(0u32));
        let action_hits = acted.clone();
        let rows: Vec<UiListRow> = (0..10)
            .map(|index| UiListRow {
                action: if index == 1 {
                    let hits = action_hits.clone();
                    Some(ListRowAction {
                        label: "X".to_string(),
                        on_activate: Rc::new(move |_| *hits.borrow_mut() += 1),
                    })
                } else {
                    None
                },
                id: format!("row{index}"),
                cells: vec![format!("Row {index}")],
                image: None,
                enabled: true,
            })
            .collect();
        let live = Rc::new(RefCell::new(Some("row0".to_string())));
        let current = live.clone();
        let stored = live.clone();
        let list = move || {
            let fired = fired.clone();
            let seen = seen.clone();
            let stored = stored.clone();
            UiControl {
                id: UiControlId::new("ui:t:list").unwrap(),
                label: "List".to_string(),
                rect: Rect {
                    x: 64.0,
                    y: 92.0,
                    width: 512.0,
                    height: 28.0,
                },
                enabled: true,
                visible: true,
                kind: UiControlKind::List {
                    row_height: Some(14.0),
                    column_widths: None,
                    on_activate: Some(Rc::new(move |_, id| fired.borrow_mut().push(id.to_string()))),
                    rows: rows.clone(),
                    selected: current.borrow().clone(),
                    on_select: Rc::new(move |_, id| {
                        seen.borrow_mut().push(id.to_string());
                        *stored.borrow_mut() = Some(id.to_string());
                    }),
                },
            }
        };
        let mut controller = NativeUiController::new(headless_options(seat.clone()));
        controller.register(
            UiMenuId::new("menu:t:list").unwrap(),
            Rc::new(move || menu("menu:t:list", "L", vec![list()])),
        );
        controller.open_menu(&UiMenuId::new("menu:t:list").unwrap()).unwrap();
        controller.draw(&context(&owner, &seat, 0)).unwrap();
        assert_eq!(
            controller.list_tops.get(&UiControlId::new("ui:t:list").unwrap()),
            Some(&0)
        );
        press(&mut controller, &seat, KeyCode::Down as i32);
        assert_eq!(selected.borrow().last().map(String::as_str), Some("row1"));
        press(&mut controller, &seat, KeyCode::PageDown as i32);
        assert_eq!(selected.borrow().last().map(String::as_str), Some("row3"));
        press(&mut controller, &seat, KeyCode::Enter as i32);
        assert_eq!(activated.borrow().as_slice(), &["row3".to_string()]);
        press(&mut controller, &seat, KeyCode::Home as i32);
        press(&mut controller, &seat, KeyCode::Down as i32);
        press(&mut controller, &seat, KeyCode::Delete as i32);
        assert_eq!(*acted.borrow(), 1);
        controller
            .input(&SeatInputEvent {
                seat: seat.clone(),
                time_ms: 0,
                kind: SeatInputEventKind::MouseWheel { delta: vec2(0.0, -1.0) },
            })
            .unwrap();
        assert_eq!(
            controller.list_tops.get(&UiControlId::new("ui:t:list").unwrap()),
            Some(&3)
        );
    }

    #[test]
    fn capture_accepts_cancels_and_supersedes() {
        let owner = owner();
        let seat = owner.seat(0);
        let mut controller = NativeUiController::new(headless_options(seat.clone()));
        controller.register(
            UiMenuId::new("menu:t:cap").unwrap(),
            Rc::new(|| menu("menu:t:cap", "C", vec![])),
        );
        controller.open_menu(&UiMenuId::new("menu:t:cap").unwrap()).unwrap();
        let accepted = Rc::new(RefCell::new(Vec::new()));
        let seen = accepted.clone();
        let cancelled = Rc::new(RefCell::new(0u32));
        let dropped = cancelled.clone();
        controller.capture_binding(
            Box::new(move |input| seen.borrow_mut().push(input)),
            Box::new(move || *dropped.borrow_mut() += 1),
        );
        assert!(controller.binding_capture());
        controller.input(&key(&seat, 97, true)).unwrap();
        assert!(!controller.binding_capture());
        assert_eq!(*accepted.borrow(), vec![PhysicalInput::Key(97)]);
        let first_cancelled = Rc::new(RefCell::new(0u32));
        let first = first_cancelled.clone();
        controller.capture_binding(Box::new(|_| {}), Box::new(move || *first.borrow_mut() += 1));
        controller.capture_binding(Box::new(|_| {}), Box::new(|| {}));
        assert_eq!(*first_cancelled.borrow(), 1);
        assert!(controller.binding_capture());
        controller.input(&key(&seat, KeyCode::Escape as i32, true)).unwrap();
        assert!(!controller.binding_capture());
        controller.capture_binding(Box::new(|_| {}), Box::new(|| {}));
        controller
            .input(&SeatInputEvent {
                seat: seat.clone(),
                time_ms: 0,
                kind: SeatInputEventKind::Focus { focused: false },
            })
            .unwrap();
        assert!(!controller.binding_capture());
        assert_eq!(*cancelled.borrow(), 0);
    }

    #[test]
    fn controller_axis_holds_and_repeats_through_draw() {
        let owner = owner();
        let seat = owner.seat(0);
        let mut controller = NativeUiController::new(headless_options(seat.clone()));
        let hits = Rc::new(RefCell::new(0u32));
        let a = button("ui:t:a", "A", 92.0, hits.clone());
        let b = button("ui:t:b", "B", 120.0, hits.clone());
        let c = button("ui:t:c", "C", 148.0, hits);
        controller.register(
            UiMenuId::new("menu:t:pad").unwrap(),
            Rc::new(move || menu("menu:t:pad", "P", vec![a.clone(), b.clone(), c.clone()])),
        );
        controller.open_menu(&UiMenuId::new("menu:t:pad").unwrap()).unwrap();
        assert_eq!(focused(&controller).as_deref(), Some("ui:t:a"));
        controller
            .input(&SeatInputEvent {
                seat: seat.clone(),
                time_ms: 0,
                kind: SeatInputEventKind::ControllerAxis {
                    device: 0,
                    axis: ControllerAxis::LeftY,
                    value: 0.8,
                },
            })
            .unwrap();
        assert_eq!(focused(&controller).as_deref(), Some("ui:t:b"));
        controller.draw(&context(&owner, &seat, 300)).unwrap();
        assert_eq!(focused(&controller).as_deref(), Some("ui:t:c"));
        controller
            .input(&SeatInputEvent {
                seat: seat.clone(),
                time_ms: 300,
                kind: SeatInputEventKind::ControllerAxis {
                    device: 0,
                    axis: ControllerAxis::LeftY,
                    value: 0.1,
                },
            })
            .unwrap();
        controller.draw(&context(&owner, &seat, 1000)).unwrap();
        assert_eq!(focused(&controller).as_deref(), Some("ui:t:c"));
    }

    #[test]
    fn draw_golden_fixture_matches_transformed_commands() {
        let owner = owner();
        let seat = owner.seat(0);
        let mut controller = NativeUiController::new(headless_options(seat.clone()));
        let hits = Rc::new(RefCell::new(0u32));
        let apply = button("ui:t:apply", "Apply", 92.0, hits);
        let toggle = UiControl {
            id: UiControlId::new("ui:t:sound").unwrap(),
            label: "Sound".to_string(),
            rect: Rect {
                x: 64.0,
                y: 120.0,
                width: 512.0,
                height: 28.0,
            },
            enabled: true,
            visible: true,
            kind: UiControlKind::Toggle {
                checked: false,
                on_change: Rc::new(|_, _| {}),
            },
        };
        controller.register(
            UiMenuId::new("menu:t:golden").unwrap(),
            Rc::new(move || menu("menu:t:golden", "Golden", vec![apply.clone(), toggle.clone()])),
        );
        controller.open_menu(&UiMenuId::new("menu:t:golden").unwrap()).unwrap();
        let commands = controller.draw(&context(&owner, &seat, 0)).unwrap();
        let font = ResourceId::new("resource:engine:font").unwrap();
        let safe = Rect {
            x: 0.0,
            y: 0.0,
            width: 640.0,
            height: 480.0,
        };
        let text = vec4(0.92, 0.88, 0.78, 1.0);
        let accent = vec4(1.0, 0.65, 0.22, 1.0);
        let panel = vec4(0.055, 0.06, 0.065, 0.94);
        let control = vec4(0.12, 0.13, 0.14, 0.9);
        let focused = vec4(0.3, 0.19, 0.09, 0.98);
        let clip = || UiDrawCommand::Clip { rect: Some(safe) };
        assert_eq!(
            commands,
            vec![
                clip(),
                UiDrawCommand::Fill {
                    rect: Rect {
                        x: 32.0,
                        y: 24.0,
                        width: 576.0,
                        height: 432.0
                    },
                    color: panel,
                },
                UiDrawCommand::Text {
                    origin: vec2(320.0, 48.0),
                    text: "Golden".to_string(),
                    font: font.clone(),
                    scale: 1.5,
                    color: accent,
                    align: TextAlign::Center,
                    shadow: true,
                },
                clip(),
                UiDrawCommand::Fill {
                    rect: Rect {
                        x: 64.0,
                        y: 92.0,
                        width: 512.0,
                        height: 26.0
                    },
                    color: focused,
                },
                UiDrawCommand::Text {
                    origin: vec2(74.0, 98.0),
                    text: "Apply".to_string(),
                    font: font.clone(),
                    scale: 1.5,
                    color: accent,
                    align: TextAlign::Left,
                    shadow: true,
                },
                clip(),
                UiDrawCommand::Fill {
                    rect: Rect {
                        x: 64.0,
                        y: 120.0,
                        width: 512.0,
                        height: 26.0
                    },
                    color: control,
                },
                UiDrawCommand::Text {
                    origin: vec2(74.0, 126.0),
                    text: "Sound".to_string(),
                    font: font.clone(),
                    scale: 1.5,
                    color: text,
                    align: TextAlign::Left,
                    shadow: true,
                },
                UiDrawCommand::Text {
                    origin: vec2(566.0, 126.0),
                    text: "Off".to_string(),
                    font: font.clone(),
                    scale: 1.5,
                    color: text,
                    align: TextAlign::Right,
                    shadow: true,
                },
                clip(),
                UiDrawCommand::Clip { rect: None },
            ]
        );
    }

    #[test]
    fn seat_mismatch_is_rejected() {
        let owner = owner();
        let seat = owner.seat(0);
        let other = owner.seat(1);
        let mut controller = NativeUiController::new(headless_options(seat.clone()));
        assert!(controller.input(&key(&other, KeyCode::Enter as i32, true)).is_err());
        assert!(controller.draw(&context(&owner, &other, 0)).is_err());
    }
}
