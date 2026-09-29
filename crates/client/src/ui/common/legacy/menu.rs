//! Team Arena menu definition parsing (`ui_shared.c`, `ui_main.c`).
//!
//! Donor provenance: `src/ui/common/legacy/menu.ts` (translated from id
//! Software's `code/ui/ui_shared.c`, `code/ui/ui_shared.h`,
//! `code/ui/ui_main.c`, and `code/cgame/cg_main.c`). This is a full,
//! synchronous, idiomatic Rust port: every donor export is present under
//! Rust case conventions.
//!
//! ## Sync policy
//!
//! The donor is `async` throughout (`register`, `publish`, `load`,
//! `parseSource`); this port is fully synchronous. Each former suspension
//! point is documented on the affected trait or method as `Promise -> sync`.
//! Host traits take `&mut self` so recording fakes stay plain structs.
//!
//! ## Memory policy
//!
//! The donor views definitions through `UiMemoryAllocation` spans owned by
//! `TeamArenaUiMemory`. That sibling (`super::team_arena::memory`) is still
//! a skeleton, so this module defines a local stand-in arena
//! ([`UiMemoryAllocation`], [`MenuArenaMemory`], [`UiStringReference`]) with
//! the exact donor field offsets. When the sibling lands, these locals
//! should be replaced by (names chosen to match the donor exports exactly):
//!
//! - `super::team_arena::memory::{UiStringReference, UiMemoryAllocation,
//!   TeamArenaUiMemory}`
//!
//! Until then [`UiMenuMemory`] is implemented here by [`MenuArenaMemory`].
//! Numeric fields are little-endian `i32`/`f32` exactly like the donor
//! `DataView`; typed pointers (strings, scripts, items, menus, resources)
//! live in a side table keyed by absolute offset.
//!
//! Definition handles ([`UiWindowDefinition`], [`UiItemDefinition`], ...)
//! are cheap clones over one allocation; the donor's frozen facades collapse
//! into the same handle. Donor-throwing getters that can only fail on
//! internal invariant violations panic with the donor message; data-state
//! overflows ([`UiListBoxDefinition::columns`],
//! [`UiItemDefinition::color_ranges`]) return `Err`, and every parse failure
//! surfaces as [`ClientError::BadUi`] with a recorded [`ScriptDiagnostic`].
//!
//! ## Float policy
//!
//! Layout and color math is `f32`, preserving the donor's `Math.fround`
//! operation order. Integer-domain values (time, counts, handles) are `i32`;
//! sizes and slot indices are `usize`.
//!
//! All failures surface as [`ClientError::BadUi`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_core::math::{Vec3, Vec4};

use super::script::preprocessor::{DebugEvalCallback, NowCallback, ReportCallback};
use super::script::{
    DiagnosticSeverity, IncludeRequest, IncludeResolver, ScriptDiagnostic, ScriptGlobalSnapshot,
    ScriptPreprocessorOptions, ScriptSource, ScriptSourcePosition, ScriptSourceReader, ScriptToken, ScriptTokenRecord,
    SourceLocation,
};
use crate::text::draw2d::Rect;
use crate::ClientError;

/// Maximum menus in the source static array (`MAX_UI_MENUS`).
pub const MAX_UI_MENUS: usize = 64;
/// Maximum items per menu in the source array (`MAX_UI_MENU_ITEMS`).
pub const MAX_UI_MENU_ITEMS: usize = 96;
/// Maximum color ranges per item (`MAX_UI_COLOR_RANGES`).
pub const MAX_UI_COLOR_RANGES: usize = 10;
/// Maximum list-box columns (`MAX_UI_LIST_COLUMNS`).
pub const MAX_UI_LIST_COLUMNS: usize = 16;
/// Maximum multi-choice rows (`MAX_UI_MULTI_CHOICES`).
pub const MAX_UI_MULTI_CHOICES: usize = 31;
/// Maximum retained script text bytes (`MAX_UI_SCRIPT_BYTES`).
pub const MAX_UI_SCRIPT_BYTES: usize = 1023;
/// Maximum HUD menu-set file bytes (`MAX_HUD_MENU_SET_BYTES`).
pub const MAX_HUD_MENU_SET_BYTES: usize = 4095;

/// Window flag bits (`UiWindowFlag`).
///
/// The donor is a numeric enum used as bitflags; the associated constants
/// below carry the same values.
pub struct UiWindowFlag(());

impl UiWindowFlag {
    /// Pointer is over the window.
    pub const MOUSE_OVER: i32 = 0x0000_0001;
    /// Window holds keyboard focus.
    pub const HAS_FOCUS: i32 = 0x0000_0002;
    /// Window is visible.
    pub const VISIBLE: i32 = 0x0000_0004;
    /// Window is greyed.
    pub const GREY: i32 = 0x0000_0008;
    /// Window is a non-focusable decoration.
    pub const DECORATION: i32 = 0x0000_0010;
    /// Window is fading out.
    pub const FADING_OUT: i32 = 0x0000_0020;
    /// Window is fading in.
    pub const FADING_IN: i32 = 0x0000_0040;
    /// Pointer is over the window text.
    pub const MOUSE_OVER_TEXT: i32 = 0x0000_0080;
    /// Window is running a rect transition.
    pub const IN_TRANSITION: i32 = 0x0000_0100;
    /// Foreground color was set explicitly.
    pub const FORE_COLOR_SET: i32 = 0x0000_0200;
    /// List box scrolls horizontally.
    pub const HORIZONTAL: i32 = 0x0000_0400;
    /// Pointer is over the list decrement control.
    pub const LIST_LEFT_ARROW: i32 = 0x0000_0800;
    /// Pointer is over the list increment control.
    pub const LIST_RIGHT_ARROW: i32 = 0x0000_1000;
    /// Pointer is over the list thumb.
    pub const LIST_THUMB: i32 = 0x0000_2000;
    /// Pointer is over the list page-up region.
    pub const LIST_PAGE_UP: i32 = 0x0000_4000;
    /// Pointer is over the list page-down region.
    pub const LIST_PAGE_DOWN: i32 = 0x0000_8000;
    /// Window is orbiting.
    pub const ORBITING: i32 = 0x0001_0000;
    /// Clicks outside the window close it.
    pub const OUT_OF_BOUNDS_CLICK: i32 = 0x0002_0000;
    /// Text wraps on carriage returns.
    pub const WRAPPED: i32 = 0x0004_0000;
    /// Text wraps automatically to the window width.
    pub const AUTO_WRAPPED: i32 = 0x0008_0000;
    /// Window paints even when invisible.
    pub const FORCED: i32 = 0x0010_0000;
    /// Window is a popup capturing the pointer.
    pub const POPUP: i32 = 0x0020_0000;
    /// Background color was set explicitly.
    pub const BACK_COLOR_SET: i32 = 0x0040_0000;
    /// Window visibility is timed.
    pub const TIMED_VISIBLE: i32 = 0x0080_0000;
}

/// Item type codes (`UiItemTypeCode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum UiItemTypeCode {
    /// Static text.
    Text = 0,
    /// Push button.
    Button = 1,
    /// Radio button.
    RadioButton = 2,
    /// Check box.
    CheckBox = 3,
    /// Text edit field.
    EditField = 4,
    /// Combo box.
    Combo = 5,
    /// List box.
    ListBox = 6,
    /// 3D model.
    Model = 7,
    /// Engine owner-draw.
    OwnerDraw = 8,
    /// Numeric edit field.
    NumericField = 9,
    /// Slider.
    Slider = 10,
    /// Yes/no toggle.
    YesNo = 11,
    /// Multi-choice.
    Multi = 12,
    /// Key binding.
    Bind = 13,
}

impl UiItemTypeCode {
    /// Decode a raw type code, or `None` for unknown codes.
    #[must_use]
    pub fn from_i32(value: i32) -> Option<UiItemTypeCode> {
        match value {
            0 => Some(UiItemTypeCode::Text),
            1 => Some(UiItemTypeCode::Button),
            2 => Some(UiItemTypeCode::RadioButton),
            3 => Some(UiItemTypeCode::CheckBox),
            4 => Some(UiItemTypeCode::EditField),
            5 => Some(UiItemTypeCode::Combo),
            6 => Some(UiItemTypeCode::ListBox),
            7 => Some(UiItemTypeCode::Model),
            8 => Some(UiItemTypeCode::OwnerDraw),
            9 => Some(UiItemTypeCode::NumericField),
            10 => Some(UiItemTypeCode::Slider),
            11 => Some(UiItemTypeCode::YesNo),
            12 => Some(UiItemTypeCode::Multi),
            13 => Some(UiItemTypeCode::Bind),
            _ => None,
        }
    }
}

/// A 640x480-space rectangle (`UiRect`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct UiRect {
    /// Origin x.
    pub x: f32,
    /// Origin y.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

impl From<&UiRect> for Rect {
    /// Convert to a draw rectangle.
    fn from(value: &UiRect) -> Rect {
        Rect {
            x: value.x,
            y: value.y,
            width: value.width,
            height: value.height,
        }
    }
}

impl From<UiRect> for Rect {
    /// Convert to a draw rectangle.
    fn from(value: UiRect) -> Rect {
        Rect::from(&value)
    }
}

impl From<&Rect> for UiRect {
    /// Convert from a draw rectangle.
    fn from(value: &Rect) -> UiRect {
        UiRect {
            x: value.x,
            y: value.y,
            width: value.width,
            height: value.height,
        }
    }
}

/// A live rectangle view into a definition allocation (`UiMutableRect`).
#[derive(Debug, Clone)]
pub struct UiMutableRect {
    alloc: UiMemoryAllocation,
    offset: usize,
}

impl UiMutableRect {
    /// View four floats at `offset` in `alloc`.
    fn new(alloc: &UiMemoryAllocation, offset: usize) -> UiMutableRect {
        UiMutableRect {
            alloc: alloc.clone(),
            offset,
        }
    }

    /// Origin x.
    #[must_use]
    pub fn x(&self) -> f32 {
        self.alloc.get_f32(self.offset)
    }

    /// Set origin x.
    pub fn set_x(&self, value: f32) {
        self.alloc.set_f32(self.offset, value);
    }

    /// Origin y.
    #[must_use]
    pub fn y(&self) -> f32 {
        self.alloc.get_f32(self.offset + 4)
    }

    /// Set origin y.
    pub fn set_y(&self, value: f32) {
        self.alloc.set_f32(self.offset + 4, value);
    }

    /// Width.
    #[must_use]
    pub fn width(&self) -> f32 {
        self.alloc.get_f32(self.offset + 8)
    }

    /// Set width.
    pub fn set_width(&self, value: f32) {
        self.alloc.set_f32(self.offset + 8, value);
    }

    /// Height.
    #[must_use]
    pub fn height(&self) -> f32 {
        self.alloc.get_f32(self.offset + 12)
    }

    /// Set height.
    pub fn set_height(&self, value: f32) {
        self.alloc.set_f32(self.offset + 12, value);
    }

    /// Snapshot the current value.
    #[must_use]
    pub fn snapshot(&self) -> UiRect {
        UiRect {
            x: self.x(),
            y: self.y(),
            width: self.width(),
            height: self.height(),
        }
    }

    /// Write a full value.
    pub fn write(&self, value: &UiRect) {
        self.set_x(value.x);
        self.set_y(value.y);
        self.set_width(value.width);
        self.set_height(value.height);
    }
}

/// A color component selector (`keyof Vec4` in the donor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UiColorComponent {
    /// X component.
    X,
    /// Y component.
    Y,
    /// Z component.
    Z,
    /// W component.
    W,
}

/// A live color view into a definition allocation (`UiMutableColor`).
#[derive(Debug, Clone)]
pub struct UiMutableColor {
    alloc: UiMemoryAllocation,
    offset: usize,
}

impl UiMutableColor {
    /// View four floats at `offset` in `alloc`.
    fn new(alloc: &UiMemoryAllocation, offset: usize) -> UiMutableColor {
        UiMutableColor {
            alloc: alloc.clone(),
            offset,
        }
    }

    /// X component.
    #[must_use]
    pub fn x(&self) -> f32 {
        self.alloc.get_f32(self.offset)
    }

    /// Set x.
    pub fn set_x(&self, value: f32) {
        self.alloc.set_f32(self.offset, value);
    }

    /// Y component.
    #[must_use]
    pub fn y(&self) -> f32 {
        self.alloc.get_f32(self.offset + 4)
    }

    /// Set y.
    pub fn set_y(&self, value: f32) {
        self.alloc.set_f32(self.offset + 4, value);
    }

    /// Z component.
    #[must_use]
    pub fn z(&self) -> f32 {
        self.alloc.get_f32(self.offset + 8)
    }

    /// Set z.
    pub fn set_z(&self, value: f32) {
        self.alloc.set_f32(self.offset + 8, value);
    }

    /// W component.
    #[must_use]
    pub fn w(&self) -> f32 {
        self.alloc.get_f32(self.offset + 12)
    }

    /// Set w.
    pub fn set_w(&self, value: f32) {
        self.alloc.set_f32(self.offset + 12, value);
    }

    /// Set one component by selector.
    pub fn set_component(&self, component: UiColorComponent, value: f32) {
        match component {
            UiColorComponent::X => self.set_x(value),
            UiColorComponent::Y => self.set_y(value),
            UiColorComponent::Z => self.set_z(value),
            UiColorComponent::W => self.set_w(value),
        }
    }

    /// Snapshot the current value.
    #[must_use]
    pub fn snapshot(&self) -> Vec4 {
        Vec4 {
            x: self.x(),
            y: self.y(),
            z: self.z(),
            w: self.w(),
        }
    }

    /// Write a full value.
    pub fn write(&self, value: &Vec4) {
        self.set_x(value.x);
        self.set_y(value.y);
        self.set_z(value.z);
        self.set_w(value.w);
    }
}

/// A live 3D-origin view into a model allocation (donor inline `origin`).
#[derive(Debug, Clone)]
pub struct UiMutableVec3 {
    alloc: UiMemoryAllocation,
    offset: usize,
}

impl UiMutableVec3 {
    /// View three floats at `offset` in `alloc`.
    fn new(alloc: &UiMemoryAllocation, offset: usize) -> UiMutableVec3 {
        UiMutableVec3 {
            alloc: alloc.clone(),
            offset,
        }
    }

    /// X component.
    #[must_use]
    pub fn x(&self) -> f32 {
        self.alloc.get_f32(self.offset)
    }

    /// Set x.
    pub fn set_x(&self, value: f32) {
        self.alloc.set_f32(self.offset, value);
    }

    /// Y component.
    #[must_use]
    pub fn y(&self) -> f32 {
        self.alloc.get_f32(self.offset + 4)
    }

    /// Set y.
    pub fn set_y(&self, value: f32) {
        self.alloc.set_f32(self.offset + 4, value);
    }

    /// Z component.
    #[must_use]
    pub fn z(&self) -> f32 {
        self.alloc.get_f32(self.offset + 8)
    }

    /// Set z.
    pub fn set_z(&self, value: f32) {
        self.alloc.set_f32(self.offset + 8, value);
    }

    /// Snapshot the current value.
    #[must_use]
    pub fn snapshot(&self) -> Vec3 {
        Vec3 {
            x: self.x(),
            y: self.y(),
            z: self.z(),
        }
    }
}

/// A shader picture reference (`UiShaderReference`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiShaderReference {
    /// Asset path, or `None` for a null registration.
    pub path: Option<String>,
}

impl UiShaderReference {
    /// Donor `kind` discriminator.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        "shader"
    }
}

/// A model reference (`UiModelReference`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiModelReference {
    /// Asset path, or `None` for a null registration.
    pub path: Option<String>,
}

impl UiModelReference {
    /// Donor `kind` discriminator.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        "model"
    }
}

/// A sound reference (`UiSoundReference`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiSoundReference {
    /// Asset path, or `None` for a null registration.
    pub path: Option<String>,
}

impl UiSoundReference {
    /// Donor `kind` discriminator.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        "sound"
    }
}

/// A font reference (`UiFontReference`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiFontReference {
    /// Asset path, or `None` for a null registration.
    pub path: Option<String>,
    /// Point size.
    pub point_size: i32,
}

impl UiFontReference {
    /// Donor `kind` discriminator.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        "font"
    }
}

/// A parsed UI script token (`UiScriptToken`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiScriptToken {
    /// Token text.
    pub text: String,
    /// Token location.
    pub location: SourceLocation,
}

/// A UI script body (`UiScript`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiScript {
    /// Rendered script text (quoted multi-character tokens, space separated).
    pub text: String,
    /// Tokens retained before truncation.
    pub tokens: Vec<UiScriptToken>,
    /// Whether the text was truncated to [`MAX_UI_SCRIPT_BYTES`].
    pub truncated: bool,
}

/// A shader, model, or sound resource stored in an allocation.
///
/// The donor spells this union inline; Rust needs the named enum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiMenuResource {
    /// Shader picture.
    Shader(UiShaderReference),
    /// Model.
    Model(UiModelReference),
    /// Sound.
    Sound(UiSoundReference),
}

impl UiMenuResource {
    /// Donor `kind` discriminator.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            UiMenuResource::Shader(_) => "shader",
            UiMenuResource::Model(_) => "model",
            UiMenuResource::Sound(_) => "sound",
        }
    }
}

// QVM32 `windowDef_t` field offsets (donor `MutableWindow`).
const WINDOW_RECT: usize = 0;
const WINDOW_CLIENT_RECT: usize = 16;
const WINDOW_NAME: usize = 32;
const WINDOW_GROUP: usize = 36;
const WINDOW_CINEMATIC: usize = 40;
const WINDOW_CINEMATIC_HANDLE: usize = 44;
const WINDOW_STYLE: usize = 48;
const WINDOW_BORDER: usize = 52;
const WINDOW_OWNER_DRAW: usize = 56;
const WINDOW_OWNER_DRAW_FLAGS: usize = 60;
const WINDOW_BORDER_SIZE: usize = 64;
const WINDOW_FLAGS: usize = 68;
const WINDOW_RECT_EFFECTS: usize = 72;
const WINDOW_RECT_EFFECTS2: usize = 88;
const WINDOW_OFFSET_TIME: usize = 104;
const WINDOW_NEXT_TIME: usize = 108;
const WINDOW_FORE_COLOR: usize = 112;
const WINDOW_BACK_COLOR: usize = 128;
const WINDOW_BORDER_COLOR: usize = 144;
const WINDOW_OUTLINE_COLOR: usize = 160;
const WINDOW_BACKGROUND: usize = 176;
const WINDOW_SIZE: usize = 180;

// `editFieldDef_t` field offsets (donor `MutableEditField`).
const EDIT_MINIMUM: usize = 0;
const EDIT_MAXIMUM: usize = 4;
const EDIT_DEFAULT: usize = 8;
const EDIT_RANGE: usize = 12;
const EDIT_MAX_CHARS: usize = 16;
const EDIT_MAX_PAINT_CHARS: usize = 20;
const EDIT_PAINT_OFFSET: usize = 24;
const EDIT_SIZE: usize = 28;

// `listBoxDef_t` field offsets (donor `MutableListBox`).
const LIST_START: usize = 0;
const LIST_END: usize = 4;
const LIST_DRAW_PADDING: usize = 8;
const LIST_CURSOR: usize = 12;
const LIST_ELEMENT_WIDTH: usize = 16;
const LIST_ELEMENT_HEIGHT: usize = 20;
const LIST_ELEMENT_STYLE: usize = 24;
const LIST_COLUMN_COUNT: usize = 28;
const LIST_COLUMNS: usize = 32;
const LIST_COLUMN_STRIDE: usize = 12;
const LIST_DOUBLE_CLICK: usize = 224;
const LIST_NOT_SELECTABLE: usize = 228;
const LIST_SIZE: usize = 232;

// `multiDef_t` field offsets (donor `MutableMulti`).
const MULTI_LABELS: usize = 0;
const MULTI_STRING_VALUES: usize = 128;
const MULTI_NUMBER_VALUES: usize = 256;
const MULTI_COUNT: usize = 384;
const MULTI_STRING_DEFINITION: usize = 388;
const MULTI_SIZE: usize = 392;
const MULTI_SLOTS: usize = 32;

// `modelDef_t` field offsets (donor `MutableModel`).
const MODEL_ANGLE: usize = 0;
const MODEL_ORIGIN: usize = 4;
const MODEL_FOV_X: usize = 16;
const MODEL_FOV_Y: usize = 20;
const MODEL_ROTATION: usize = 24;
const MODEL_SIZE: usize = 28;

// QVM32 `itemDef_t` field offsets (donor `MutableItem`).
const ITEM_WINDOW: usize = 0;
const ITEM_TEXT_RECT: usize = 180;
const ITEM_TYPE: usize = 196;
const ITEM_ALIGNMENT: usize = 200;
const ITEM_TEXT_ALIGNMENT: usize = 204;
const ITEM_TEXT_ALIGN_X: usize = 208;
const ITEM_TEXT_ALIGN_Y: usize = 212;
const ITEM_TEXT_SCALE: usize = 216;
const ITEM_TEXT_STYLE: usize = 220;
const ITEM_TEXT: usize = 224;
const ITEM_PARENT: usize = 228;
const ITEM_ASSET: usize = 232;
const ITEM_MOUSE_ENTER_TEXT: usize = 236;
const ITEM_MOUSE_EXIT_TEXT: usize = 240;
const ITEM_MOUSE_ENTER: usize = 244;
const ITEM_MOUSE_EXIT: usize = 248;
const ITEM_ACTION: usize = 252;
const ITEM_ON_FOCUS: usize = 256;
const ITEM_LEAVE_FOCUS: usize = 260;
const ITEM_CVAR: usize = 264;
const ITEM_CVAR_TEST: usize = 268;
const ITEM_CVAR_SCRIPT: usize = 272;
const ITEM_CVAR_FLAGS: usize = 276;
const ITEM_FOCUS_SOUND: usize = 280;
const ITEM_COLOR_COUNT: usize = 284;
const ITEM_COLOR_RANGES: usize = 288;
const ITEM_COLOR_STRIDE: usize = 24;
const ITEM_SPECIAL: usize = 528;
const ITEM_CURSOR: usize = 532;
const ITEM_TYPE_DATA: usize = 536;
const ITEM_SIZE: usize = 540;

// QVM32 `menuDef_t` field offsets (donor `MutableMenu`).
const MENU_WINDOW: usize = 0;
const MENU_FONT: usize = 180;
const MENU_FULL_SCREEN: usize = 184;
const MENU_ITEM_COUNT: usize = 188;
const MENU_FONT_INDEX: usize = 192;
const MENU_CURSOR_ITEM: usize = 196;
const MENU_FADE_CYCLE: usize = 200;
const MENU_FADE_CLAMP: usize = 204;
const MENU_FADE_AMOUNT: usize = 208;
const MENU_ON_OPEN: usize = 212;
const MENU_ON_CLOSE: usize = 216;
const MENU_ON_ESCAPE: usize = 220;
const MENU_SOUND_LOOP: usize = 224;
const MENU_FOCUS_COLOR: usize = 228;
const MENU_DISABLE_COLOR: usize = 244;
const MENU_ITEMS: usize = 260;
const MENU_SIZE: usize = 644;

// `cvarFlags` rule bits (donor `cvarRule`).
const CVAR_RULE_ENABLE: i32 = 1;
const CVAR_RULE_DISABLE: i32 = 2;
const CVAR_RULE_SHOW: i32 = 4;
const CVAR_RULE_HIDE: i32 = 8;
const CVAR_RULE_MASK: i32 = 15;

/// A char pointer into retained byte storage (`UiStringReference`).
///
/// Donor-derived stand-in for `super::team_arena::memory::UiStringReference`;
/// see the module docs. Storage is append-only, so offsets stay valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiStringReference {
    bytes: Rc<RefCell<Vec<u8>>>,
    offset: usize,
}

impl UiStringReference {
    /// Build a reference at `offset` (must be inside `bytes`).
    fn with_storage(bytes: Rc<RefCell<Vec<u8>>>, offset: usize) -> UiStringReference {
        assert!(
            offset < bytes.borrow().len(),
            "UI string pointer is outside its byte storage"
        );
        UiStringReference { bytes, offset }
    }

    /// Build a detached NUL-terminated copy of `text` (truncated at NUL).
    pub fn literal(text: &str) -> Result<UiStringReference, ClientError> {
        let end = text.find('\0').unwrap_or(text.len());
        let mut bytes = Vec::with_capacity(end + 1);
        for ch in text[..end].chars() {
            let byte = ch as u32;
            if byte > 255 {
                return Err(ClientError::BadUi("UI string requires source bytes".to_string()));
            }
            bytes.push(byte as u8);
        }
        bytes.push(0);
        Ok(UiStringReference {
            bytes: Rc::new(RefCell::new(bytes)),
            offset: 0,
        })
    }

    /// Read the NUL-terminated Latin-1 string.
    #[must_use]
    pub fn read(&self) -> String {
        let bytes = self.bytes.borrow();
        let mut result = String::new();
        let mut index = self.offset;
        while index < bytes.len() {
            let byte = bytes[index];
            if byte == 0 {
                return result;
            }
            result.push(char::from(byte));
            index += 1;
        }
        panic!("UI string reads beyond its retained byte storage");
    }
}
/// A typed pointer stored beside the raw bytes (donor `UiMemoryPointer`).
#[derive(Debug, Clone)]
enum UiMemoryPointer {
    /// String pointer.
    String(UiStringReference),
    /// Nested allocation pointer.
    Allocation(UiMemoryAllocation),
    /// Retained script.
    Script(UiScript),
    /// Retained item definition.
    Item(UiItemDefinition),
    /// Retained menu definition.
    Menu(UiMenuDefinition),
    /// Registered resource plus optional handle.
    Resource {
        /// Resource value.
        value: UiMenuResource,
        /// Registration handle.
        handle: Option<i32>,
    },
}

/// Raw bytes plus the typed-pointer side table.
#[derive(Debug)]
struct MemoryArena {
    bytes: Vec<u8>,
    pointers: HashMap<usize, UiMemoryPointer>,
}

impl MemoryArena {
    /// Zeroed arena of `size` bytes.
    fn zeroed(size: usize) -> MemoryArena {
        MemoryArena {
            bytes: vec![0; size],
            pointers: HashMap::new(),
        }
    }
}

/// A physical `UI_Alloc` span (`UiMemoryAllocation`).
///
/// Donor-derived stand-in for
/// `super::team_arena::memory::UiMemoryAllocation`; see the module docs.
/// Typed pointers deliberately have no fabricated QVM address bits, exactly
/// like the donor.
#[derive(Debug, Clone)]
pub struct UiMemoryAllocation {
    arena: Rc<RefCell<MemoryArena>>,
    offset: usize,
    size: usize,
}

impl UiMemoryAllocation {
    /// Detached zeroed span (donor `UiMemoryAllocation.zeroed`).
    pub fn zeroed(size: usize) -> UiMemoryAllocation {
        UiMemoryAllocation {
            arena: Rc::new(RefCell::new(MemoryArena::zeroed(size))),
            offset: 0,
            size,
        }
    }

    /// Absolute offset of this span in its arena.
    #[must_use]
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Whether two spans view the same arena bytes (donor `===` on records).
    pub(crate) fn same_span(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.arena, &other.arena) && self.offset == other.offset && self.size == other.size
    }

    /// Span length in bytes.
    #[must_use]
    pub fn size(&self) -> usize {
        self.size
    }

    /// View a sub-record (donor `subrecord`).
    pub(crate) fn subrecord(&self, offset: usize, size: usize) -> UiMemoryAllocation {
        if offset + size > self.size {
            panic!("UI record view is outside its retained storage");
        }
        UiMemoryAllocation {
            arena: Rc::clone(&self.arena),
            offset: self.offset + offset,
            size,
        }
    }

    /// Reinterpret this span with a new size (donor `dereference`).
    pub(crate) fn dereference(&self, size: usize) -> UiMemoryAllocation {
        UiMemoryAllocation {
            arena: Rc::clone(&self.arena),
            offset: self.offset,
            size,
        }
    }

    /// Zero the span and drop overlapping pointers (donor `clear`).
    pub(crate) fn clear(&self) {
        self.discard_pointers(0, self.size);
        let mut arena = self.arena.borrow_mut();
        arena.bytes[self.offset..self.offset + self.size].fill(0);
    }

    /// Read a little-endian `i32` (donor `getInt32`).
    pub(crate) fn get_i32(&self, offset: usize) -> i32 {
        self.check_numeric(offset);
        i32::from_le_bytes(self.raw_word(offset))
    }

    /// Read a little-endian `f32` (donor `getFloat32`).
    pub(crate) fn get_f32(&self, offset: usize) -> f32 {
        self.check_numeric(offset);
        f32::from_le_bytes(self.raw_word(offset))
    }

    /// Write a little-endian `i32` (donor `setInt32`).
    pub(crate) fn set_i32(&self, offset: usize, value: i32) {
        self.check_word(offset);
        self.discard_pointers(offset, 4);
        let mut arena = self.arena.borrow_mut();
        let at = self.offset + offset;
        arena.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// Write a little-endian `f32` (donor `setFloat32`).
    pub(crate) fn set_f32(&self, offset: usize, value: f32) {
        self.check_word(offset);
        self.discard_pointers(offset, 4);
        let mut arena = self.arena.borrow_mut();
        let at = self.offset + offset;
        arena.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// Read a string pointer (donor `getString`).
    pub(crate) fn get_string(&self, offset: usize) -> Option<String> {
        self.get_string_reference(offset).map(|value| value.read())
    }

    /// Read a raw string reference (donor `getStringReference`).
    pub(crate) fn get_string_reference(&self, offset: usize) -> Option<UiStringReference> {
        match self.pointer(offset) {
            Some(UiMemoryPointer::String(value)) => Some(value),
            Some(_) => panic!("UI_Alloc string read aliases a non-string pointer"),
            None => None,
        }
    }

    /// Write a string pointer (donor `setString`).
    pub(crate) fn set_string(&self, offset: usize, value: Option<UiStringReference>) {
        self.set_pointer(offset, value.map(UiMemoryPointer::String));
    }

    /// Write a nested allocation pointer (donor `setAllocationPointer`).
    pub(crate) fn set_allocation_pointer(&self, offset: usize, value: Option<UiMemoryAllocation>) {
        self.set_pointer(offset, value.map(UiMemoryPointer::Allocation));
    }

    /// Whether the word holds a null pointer (donor `isNullPointer`).
    pub(crate) fn is_null_pointer(&self, offset: usize) -> bool {
        self.check_word(offset);
        let absolute = self.offset + offset;
        let pointer = self.arena.borrow().pointers.get(&absolute).cloned();
        match pointer {
            None => self.raw_u32(offset) == 0,
            Some(UiMemoryPointer::Resource { handle: Some(_), .. }) => self.raw_u32(offset) == 0,
            Some(_) => false,
        }
    }

    /// Read a nested allocation pointer (donor `getAllocationPointer`).
    pub(crate) fn get_allocation_pointer(&self, offset: usize) -> Option<UiMemoryAllocation> {
        match self.pointer(offset) {
            None => None,
            Some(UiMemoryPointer::Allocation(value)) => Some(value),
            Some(_) => panic!("UI allocation read aliases a different typed pointer"),
        }
    }

    /// Read a retained script (donor `getScript`).
    pub(crate) fn get_script(&self, offset: usize) -> Option<UiScript> {
        match self.pointer(offset) {
            None => None,
            Some(UiMemoryPointer::Script(value)) => Some(value),
            Some(_) => panic!("UI script read aliases a different typed pointer"),
        }
    }

    /// Write a retained script (donor `setScript`).
    pub(crate) fn set_script(&self, offset: usize, value: Option<UiScript>) {
        self.set_pointer(offset, value.map(UiMemoryPointer::Script));
    }

    /// Read a retained item definition (donor `getItem`).
    pub(crate) fn get_item(&self, offset: usize) -> Option<UiItemDefinition> {
        match self.pointer(offset) {
            None => None,
            Some(UiMemoryPointer::Item(value)) => Some(value),
            Some(_) => panic!("UI item read aliases a different typed pointer"),
        }
    }

    /// Write a retained item definition (donor `setItem`).
    pub(crate) fn set_item(&self, offset: usize, value: Option<UiItemDefinition>) {
        self.set_pointer(offset, value.map(UiMemoryPointer::Item));
    }

    /// Read a retained menu definition (donor `getMenu`).
    pub(crate) fn get_menu(&self, offset: usize) -> Option<UiMenuDefinition> {
        match self.pointer(offset) {
            None => None,
            Some(UiMemoryPointer::Menu(value)) => Some(value),
            Some(_) => panic!("UI menu read aliases a different typed pointer"),
        }
    }

    /// Write a retained menu definition (donor `setMenu`).
    pub(crate) fn set_menu(&self, offset: usize, value: Option<UiMenuDefinition>) {
        self.set_pointer(offset, value.map(UiMemoryPointer::Menu));
    }

    /// Read a retained resource (donor `getResource`).
    pub(crate) fn get_resource(&self, offset: usize) -> Option<UiMenuResource> {
        self.check_word(offset);
        let absolute = self.offset + offset;
        match self.arena.borrow().pointers.get(&absolute).cloned() {
            Some(UiMemoryPointer::Resource { value, .. }) => Some(value),
            None => {
                self.check_numeric(offset);
                None
            }
            Some(_) => panic!("UI resource handle read aliases a different typed pointer"),
        }
    }

    /// Read a retained resource handle (donor `getResourceHandle`).
    pub(crate) fn get_resource_handle(&self, offset: usize) -> Option<i32> {
        self.check_word(offset);
        let absolute = self.offset + offset;
        match self.arena.borrow().pointers.get(&absolute).cloned() {
            Some(UiMemoryPointer::Resource { handle: None, .. }) => None,
            _ => Some(self.get_i32(offset)),
        }
    }

    /// Write a retained resource plus handle (donor `setResource`).
    pub(crate) fn set_resource(&self, offset: usize, value: UiMenuResource, handle: Option<i32>) {
        self.set_i32(offset, handle.unwrap_or(0));
        let absolute = self.offset + offset;
        self.arena
            .borrow_mut()
            .pointers
            .insert(absolute, UiMemoryPointer::Resource { value, handle });
    }

    /// Read a pointer cell, rejecting nonzero untyped bytes (donor `pointer`).
    fn pointer(&self, offset: usize) -> Option<UiMemoryPointer> {
        self.check_word(offset);
        let absolute = self.offset + offset;
        match self.arena.borrow().pointers.get(&absolute).cloned() {
            None => {
                self.check_numeric(offset);
                if self.raw_u32(offset) != 0 {
                    panic!("UI pointer read contains non-pointer bytes");
                }
                None
            }
            Some(UiMemoryPointer::Resource { handle: Some(_), .. }) => {
                self.check_numeric(offset);
                if self.raw_u32(offset) != 0 {
                    panic!("UI pointer read contains non-pointer bytes");
                }
                None
            }
            other => other,
        }
    }

    /// Write a pointer cell and zero its address bytes (donor `setPointer`).
    fn set_pointer(&self, offset: usize, pointer: Option<UiMemoryPointer>) {
        self.set_i32(offset, 0);
        if let Some(pointer) = pointer {
            let absolute = self.offset + offset;
            self.arena.borrow_mut().pointers.insert(absolute, pointer);
        }
    }

    /// Bounds-check a word access (donor `checkWord`).
    fn check_word(&self, offset: usize) {
        if offset + 4 > self.size {
            panic!("UI_Alloc field is outside the borrowed record");
        }
    }

    /// Reject numeric reads over typed pointers (donor `checkNumeric`).
    fn check_numeric(&self, offset: usize) {
        self.check_word(offset);
        let arena = self.arena.borrow();
        let absolute = self.offset + offset;
        let start = absolute.saturating_sub(3);
        for address in start..absolute + 4 {
            match arena.pointers.get(&address) {
                None => {}
                Some(UiMemoryPointer::Resource { handle: Some(_), .. }) => {}
                Some(_) => panic!("UI_Alloc numeric read requires QVM pointer address bits"),
            }
        }
    }

    /// Drop pointers overlapping a byte range (donor `discardPointers`).
    fn discard_pointers(&self, offset: usize, size: usize) {
        if size == 0 {
            return;
        }
        let mut arena = self.arena.borrow_mut();
        let absolute = self.offset + offset;
        let start = absolute.saturating_sub(3);
        for address in start..absolute + size {
            arena.pointers.remove(&address);
        }
    }

    /// Raw word bytes without pointer checks.
    fn raw_word(&self, offset: usize) -> [u8; 4] {
        let arena = self.arena.borrow();
        let at = self.offset + offset;
        [
            arena.bytes[at],
            arena.bytes[at + 1],
            arena.bytes[at + 2],
            arena.bytes[at + 3],
        ]
    }

    /// Raw `u32` without pointer checks.
    fn raw_u32(&self, offset: usize) -> u32 {
        u32::from_le_bytes(self.raw_word(offset))
    }
}

/// `UI_Alloc` pool consumption counters.
#[derive(Debug, Default)]
struct ArenaPoints {
    string_point: usize,
    allocation_point: usize,
    exhausted: bool,
}

/// `UI_Alloc`/`String_Alloc` memory (`TeamArenaUiMemory` donor behavior).
///
/// Donor-derived stand-in for `super::team_arena::memory::TeamArenaUiMemory`;
/// see the module docs. Sizes match the donor `ui` module (1 MiB pool,
/// 384 KiB string pool, 64 static 644-byte menu records); allocations are
/// 16-byte aligned. String interning buckets are omitted: every allocation
/// appends to the pool, which is unobservable except through pool-full
/// (`None`) behavior.
#[derive(Debug, Clone)]
pub struct MenuArenaMemory {
    main: Rc<RefCell<MemoryArena>>,
    menus: Rc<RefCell<MemoryArena>>,
    points: Rc<RefCell<ArenaPoints>>,
    strings: Rc<RefCell<Vec<u8>>>,
    string_capacity: usize,
}

impl MenuArenaMemory {
    /// Donor `ui`-module pool sizes.
    pub fn new() -> MenuArenaMemory {
        MenuArenaMemory::with_pool_sizes(1024 * 1024, 384 * 1024)
    }

    /// Pools with explicit sizes (small sizes drive out-of-memory tests).
    pub fn with_pool_sizes(main_bytes: usize, string_bytes: usize) -> MenuArenaMemory {
        MenuArenaMemory {
            main: Rc::new(RefCell::new(MemoryArena::zeroed(main_bytes))),
            menus: Rc::new(RefCell::new(MemoryArena::zeroed(MAX_UI_MENUS * MENU_SIZE))),
            points: Rc::new(RefCell::new(ArenaPoints::default())),
            strings: Rc::new(RefCell::new(vec![0; string_bytes.max(1)])),
            string_capacity: string_bytes.max(1),
        }
    }

    /// Bytes consumed from the string pool.
    #[must_use]
    pub fn string_bytes(&self) -> usize {
        self.points.borrow().string_point
    }

    /// Bytes consumed from the allocation pool.
    #[must_use]
    pub fn allocated_bytes(&self) -> usize {
        self.points.borrow().allocation_point
    }

    /// Whether an allocation has failed.
    #[must_use]
    pub fn out_of_memory(&self) -> bool {
        self.points.borrow().exhausted
    }
}

impl Default for MenuArenaMemory {
    /// Donor `ui`-module pool sizes.
    fn default() -> MenuArenaMemory {
        MenuArenaMemory::new()
    }
}

/// `UI_Alloc` surface consumed by the menu parser (`UiMenuMemory`).
pub trait UiMenuMemory: std::fmt::Debug {
    /// Allocate `size` bytes, or `None` when the pool is exhausted.
    fn allocate(&self, size: usize) -> Option<usize>;
    /// Borrow a span of the main pool.
    fn borrow(&self, offset: usize, size: usize) -> UiMemoryAllocation;
    /// Borrow static menu record `index`.
    fn menu_record(&self, index: usize) -> UiMemoryAllocation;
    /// Copy text into the string pool (donor `stringAlloc`).
    fn string_alloc(&self, text: Option<&str>) -> Result<Option<String>, ClientError>;
    /// Reference text in the string pool (donor `stringAllocReference`).
    fn string_alloc_reference(&self, text: Option<&str>) -> Result<Option<UiStringReference>, ClientError>;
}

impl UiMenuMemory for MenuArenaMemory {
    /// Allocate with 16-byte alignment (donor `allocate`).
    fn allocate(&self, size: usize) -> Option<usize> {
        let mut points = self.points.borrow_mut();
        if points.allocation_point + size > self.main.borrow().bytes.len() {
            points.exhausted = true;
            return None;
        }
        let offset = points.allocation_point;
        points.allocation_point += (size + 15) & !15;
        Some(offset)
    }

    /// Borrow a main-pool span.
    fn borrow(&self, offset: usize, size: usize) -> UiMemoryAllocation {
        if self.main.borrow().bytes.len() < offset + size {
            panic!("UI_Alloc borrow is outside the physical memory pool");
        }
        UiMemoryAllocation {
            arena: Rc::clone(&self.main),
            offset,
            size,
        }
    }

    /// Borrow static menu record `index` (donor `menuRecord`).
    fn menu_record(&self, index: usize) -> UiMemoryAllocation {
        if index >= MAX_UI_MENUS {
            panic!("UI menu index exceeds the source static array");
        }
        UiMemoryAllocation {
            arena: Rc::clone(&self.menus),
            offset: index * MENU_SIZE,
            size: MENU_SIZE,
        }
    }

    /// Copy text into the string pool.
    fn string_alloc(&self, text: Option<&str>) -> Result<Option<String>, ClientError> {
        match self.string_alloc_reference(text)? {
            Some(reference) => Ok(Some(reference.read())),
            None => Ok(None),
        }
    }

    /// Reference text in the string pool.
    fn string_alloc_reference(&self, text: Option<&str>) -> Result<Option<UiStringReference>, ClientError> {
        let Some(text) = text else {
            return Ok(None);
        };
        let end = text.find('\0').unwrap_or(text.len());
        let text = &text[..end];
        if text.is_empty() {
            return Ok(Some(UiStringReference::literal("")?));
        }
        let mut bytes = Vec::with_capacity(text.len());
        for ch in text.chars() {
            let byte = ch as u32;
            if byte > 255 {
                return Err(ClientError::BadUi(
                    "String_Alloc requires source byte strings".to_string(),
                ));
            }
            bytes.push(byte as u8);
        }
        let mut points = self.points.borrow_mut();
        if bytes.len() + points.string_point + 1 >= self.string_capacity {
            return Ok(None);
        }
        let offset = points.string_point;
        {
            let mut strings = self.strings.borrow_mut();
            strings[offset..offset + bytes.len()].copy_from_slice(&bytes);
            strings[offset + bytes.len()] = 0;
        }
        points.string_point += bytes.len() + 1;
        Ok(Some(UiStringReference::with_storage(Rc::clone(&self.strings), offset)))
    }
}

/// Shared `UI_Alloc` memory handle (`UiMenuMemory` ownership level).
///
/// Cloning shares the pools; trait methods use interior mutability so the
/// handle stays object-safe for future sibling implementations.
#[derive(Debug, Clone)]
pub struct SharedUiMenuMemory {
    inner: Rc<dyn UiMenuMemory>,
}

impl SharedUiMenuMemory {
    /// Share any memory implementation.
    pub fn new(memory: impl UiMenuMemory + 'static) -> SharedUiMenuMemory {
        SharedUiMenuMemory { inner: Rc::new(memory) }
    }

    /// Whether two handles share the same pools (donor pointer equality).
    #[must_use]
    pub fn same_memory(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }
}

impl UiMenuMemory for SharedUiMenuMemory {
    /// Allocate through the shared memory.
    fn allocate(&self, size: usize) -> Option<usize> {
        self.inner.allocate(size)
    }

    /// Borrow through the shared memory.
    fn borrow(&self, offset: usize, size: usize) -> UiMemoryAllocation {
        self.inner.borrow(offset, size)
    }

    /// Borrow a menu record through the shared memory.
    fn menu_record(&self, index: usize) -> UiMemoryAllocation {
        self.inner.menu_record(index)
    }

    /// Allocate a string through the shared memory.
    fn string_alloc(&self, text: Option<&str>) -> Result<Option<String>, ClientError> {
        self.inner.string_alloc(text)
    }

    /// Allocate a string reference through the shared memory.
    fn string_alloc_reference(&self, text: Option<&str>) -> Result<Option<UiStringReference>, ClientError> {
        self.inner.string_alloc_reference(text)
    }
}

/// Memory ownership carried with parsed definitions (`UiMenuMemoryOwnership`).
#[derive(Debug, Clone)]
pub enum UiMenuMemoryOwnership {
    /// Definitions use detached literals; nothing is accounted.
    Unaccounted,
    /// Definitions are backed by shared QVM32 pools.
    Qvm32 {
        /// Shared pools.
        memory: SharedUiMenuMemory,
    },
}

impl UiMenuMemoryOwnership {
    /// Donor `kind` discriminator.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            UiMenuMemoryOwnership::Unaccounted => "unaccounted",
            UiMenuMemoryOwnership::Qvm32 { .. } => "qvm32",
        }
    }
}
/// A window definition view over a 180-byte record (`UiWindowDefinition`).
#[derive(Debug, Clone)]
pub struct UiWindowDefinition {
    alloc: UiMemoryAllocation,
}

impl UiWindowDefinition {
    /// View a window record.
    fn new(alloc: UiMemoryAllocation) -> UiWindowDefinition {
        UiWindowDefinition { alloc }
    }

    /// Zero the record and apply donor defaults (donor `initialize`).
    pub(crate) fn initialize(&self) {
        self.alloc.clear();
        self.set_border_size(1.0);
        self.set_fore_color(&Vec4 {
            x: 1.0,
            y: 1.0,
            z: 1.0,
            w: 1.0,
        });
        self.alloc.set_i32(WINDOW_CINEMATIC_HANDLE, -1);
    }

    /// Screen rectangle (live view).
    #[must_use]
    pub fn rect(&self) -> UiMutableRect {
        UiMutableRect::new(&self.alloc, WINDOW_RECT)
    }

    /// Write the screen rectangle.
    pub fn set_rect(&self, value: &UiRect) {
        self.rect().write(value);
    }

    /// Client rectangle (live view).
    #[must_use]
    pub fn client_rect(&self) -> UiMutableRect {
        UiMutableRect::new(&self.alloc, WINDOW_CLIENT_RECT)
    }

    /// Write the client rectangle.
    pub fn set_client_rect(&self, value: &UiRect) {
        self.client_rect().write(value);
    }

    /// Effects rectangle (live view).
    #[must_use]
    pub fn rect_effects(&self) -> UiMutableRect {
        UiMutableRect::new(&self.alloc, WINDOW_RECT_EFFECTS)
    }

    /// Write the effects rectangle.
    pub fn set_rect_effects(&self, value: &UiRect) {
        self.rect_effects().write(value);
    }

    /// Second effects rectangle (live view).
    #[must_use]
    pub fn rect_effects2(&self) -> UiMutableRect {
        UiMutableRect::new(&self.alloc, WINDOW_RECT_EFFECTS2)
    }

    /// Write the second effects rectangle.
    pub fn set_rect_effects2(&self, value: &UiRect) {
        self.rect_effects2().write(value);
    }

    /// Window name.
    #[must_use]
    pub fn name(&self) -> Option<String> {
        self.alloc.get_string(WINDOW_NAME)
    }

    /// Write the window name.
    pub(crate) fn set_name(&self, value: Option<UiStringReference>) {
        self.alloc.set_string(WINDOW_NAME, value);
    }

    /// Window group.
    #[must_use]
    pub fn group(&self) -> Option<String> {
        self.alloc.get_string(WINDOW_GROUP)
    }

    /// Write the window group.
    pub(crate) fn set_group(&self, value: Option<UiStringReference>) {
        self.alloc.set_string(WINDOW_GROUP, value);
    }

    /// Cinematic path.
    #[must_use]
    pub fn cinematic(&self) -> Option<String> {
        self.alloc.get_string(WINDOW_CINEMATIC)
    }

    /// Write the cinematic path.
    pub(crate) fn set_cinematic(&self, value: Option<UiStringReference>) {
        self.alloc.set_string(WINDOW_CINEMATIC, value);
    }

    /// Window style.
    #[must_use]
    pub fn style(&self) -> i32 {
        self.alloc.get_i32(WINDOW_STYLE)
    }

    /// Write the window style.
    pub(crate) fn set_style(&self, value: i32) {
        self.alloc.set_i32(WINDOW_STYLE, value);
    }

    /// Border style.
    #[must_use]
    pub fn border(&self) -> i32 {
        self.alloc.get_i32(WINDOW_BORDER)
    }

    /// Write the border style.
    pub(crate) fn set_border(&self, value: i32) {
        self.alloc.set_i32(WINDOW_BORDER, value);
    }

    /// Owner-draw id.
    #[must_use]
    pub fn owner_draw(&self) -> i32 {
        self.alloc.get_i32(WINDOW_OWNER_DRAW)
    }

    /// Write the owner-draw id.
    pub(crate) fn set_owner_draw(&self, value: i32) {
        self.alloc.set_i32(WINDOW_OWNER_DRAW, value);
    }

    /// Owner-draw flags.
    #[must_use]
    pub fn owner_draw_flags(&self) -> i32 {
        self.alloc.get_i32(WINDOW_OWNER_DRAW_FLAGS)
    }

    /// Write the owner-draw flags.
    pub(crate) fn set_owner_draw_flags(&self, value: i32) {
        self.alloc.set_i32(WINDOW_OWNER_DRAW_FLAGS, value);
    }

    /// Border size.
    #[must_use]
    pub fn border_size(&self) -> f32 {
        self.alloc.get_f32(WINDOW_BORDER_SIZE)
    }

    /// Write the border size.
    pub(crate) fn set_border_size(&self, value: f32) {
        self.alloc.set_f32(WINDOW_BORDER_SIZE, value);
    }

    /// Window flags.
    #[must_use]
    pub fn flags(&self) -> i32 {
        self.alloc.get_i32(WINDOW_FLAGS)
    }

    /// Write the window flags.
    pub fn set_flags(&self, value: i32) {
        self.alloc.set_i32(WINDOW_FLAGS, value);
    }

    /// Next effect time.
    #[must_use]
    pub fn next_time(&self) -> i32 {
        self.alloc.get_i32(WINDOW_NEXT_TIME)
    }

    /// Write the next effect time.
    pub fn set_next_time(&self, value: i32) {
        self.alloc.set_i32(WINDOW_NEXT_TIME, value);
    }

    /// Effect offset time.
    #[must_use]
    pub fn offset_time(&self) -> i32 {
        self.alloc.get_i32(WINDOW_OFFSET_TIME)
    }

    /// Write the effect offset time.
    pub fn set_offset_time(&self, value: i32) {
        self.alloc.set_i32(WINDOW_OFFSET_TIME, value);
    }

    /// Cinematic handle.
    #[must_use]
    pub fn cinematic_handle(&self) -> i32 {
        self.alloc.get_i32(WINDOW_CINEMATIC_HANDLE)
    }

    /// Write the cinematic handle.
    pub fn set_cinematic_handle(&self, value: i32) {
        self.alloc.set_i32(WINDOW_CINEMATIC_HANDLE, value);
    }

    /// Foreground color (live view).
    #[must_use]
    pub fn fore_color(&self) -> UiMutableColor {
        UiMutableColor::new(&self.alloc, WINDOW_FORE_COLOR)
    }

    /// Write the foreground color.
    pub fn set_fore_color(&self, value: &Vec4) {
        self.fore_color().write(value);
    }

    /// Background color (live view).
    #[must_use]
    pub fn back_color(&self) -> UiMutableColor {
        UiMutableColor::new(&self.alloc, WINDOW_BACK_COLOR)
    }

    /// Write the background color.
    pub fn set_back_color(&self, value: &Vec4) {
        self.back_color().write(value);
    }

    /// Border color (live view).
    #[must_use]
    pub fn border_color(&self) -> UiMutableColor {
        UiMutableColor::new(&self.alloc, WINDOW_BORDER_COLOR)
    }

    /// Write the border color.
    pub fn set_border_color(&self, value: &Vec4) {
        self.border_color().write(value);
    }

    /// Outline color (live view).
    #[must_use]
    pub fn outline_color(&self) -> UiMutableColor {
        UiMutableColor::new(&self.alloc, WINDOW_OUTLINE_COLOR)
    }

    /// Write the outline color.
    pub fn set_outline_color(&self, value: &Vec4) {
        self.outline_color().write(value);
    }

    /// Background shader reference.
    #[must_use]
    pub fn background(&self) -> Option<UiShaderReference> {
        match self.alloc.get_resource(WINDOW_BACKGROUND) {
            None => None,
            Some(UiMenuResource::Shader(value)) => Some(value),
            Some(_) => panic!("UI window background aliases a different resource kind"),
        }
    }

    /// Background shader handle.
    #[must_use]
    pub fn background_handle(&self) -> Option<i32> {
        self.alloc.get_resource_handle(WINDOW_BACKGROUND)
    }

    /// Write the background shader plus handle (donor `setBackground`).
    pub fn set_background(&self, value: UiShaderReference, handle: Option<i32>) {
        self.alloc
            .set_resource(WINDOW_BACKGROUND, UiMenuResource::Shader(value), handle);
    }
}

/// A color range value (`UiColorRange`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiColorRange {
    /// Low bound.
    pub low: f32,
    /// High bound.
    pub high: f32,
    /// Range color.
    pub color: Vec4,
}

/// An edit-field definition view (`UiEditFieldDefinition`).
#[derive(Debug, Clone)]
pub struct UiEditFieldDefinition {
    alloc: UiMemoryAllocation,
}

impl UiEditFieldDefinition {
    /// View an edit-field record.
    fn new(alloc: UiMemoryAllocation) -> UiEditFieldDefinition {
        UiEditFieldDefinition { alloc }
    }

    /// Minimum value.
    #[must_use]
    pub fn minimum(&self) -> f32 {
        self.alloc.get_f32(EDIT_MINIMUM)
    }

    /// Write the minimum value.
    pub(crate) fn set_minimum(&self, value: f32) {
        self.alloc.set_f32(EDIT_MINIMUM, value);
    }

    /// Maximum value.
    #[must_use]
    pub fn maximum(&self) -> f32 {
        self.alloc.get_f32(EDIT_MAXIMUM)
    }

    /// Write the maximum value.
    pub(crate) fn set_maximum(&self, value: f32) {
        self.alloc.set_f32(EDIT_MAXIMUM, value);
    }

    /// Default value.
    #[must_use]
    pub fn default_value(&self) -> f32 {
        self.alloc.get_f32(EDIT_DEFAULT)
    }

    /// Write the default value.
    pub(crate) fn set_default_value(&self, value: f32) {
        self.alloc.set_f32(EDIT_DEFAULT, value);
    }

    /// Value range.
    #[must_use]
    pub fn range(&self) -> f32 {
        self.alloc.get_f32(EDIT_RANGE)
    }

    /// Write the value range (donor setter; the parser never calls it).
    #[allow(dead_code)]
    pub(crate) fn set_range(&self, value: f32) {
        self.alloc.set_f32(EDIT_RANGE, value);
    }

    /// Maximum characters.
    #[must_use]
    pub fn max_chars(&self) -> i32 {
        self.alloc.get_i32(EDIT_MAX_CHARS)
    }

    /// Write the maximum characters.
    pub(crate) fn set_max_chars(&self, value: i32) {
        self.alloc.set_i32(EDIT_MAX_CHARS, value);
    }

    /// Maximum painted characters.
    #[must_use]
    pub fn max_paint_chars(&self) -> i32 {
        self.alloc.get_i32(EDIT_MAX_PAINT_CHARS)
    }

    /// Write the maximum painted characters.
    pub(crate) fn set_max_paint_chars(&self, value: i32) {
        self.alloc.set_i32(EDIT_MAX_PAINT_CHARS, value);
    }

    /// Paint offset.
    #[must_use]
    pub fn paint_offset(&self) -> i32 {
        self.alloc.get_i32(EDIT_PAINT_OFFSET)
    }

    /// Write the paint offset.
    pub fn set_paint_offset(&self, value: i32) {
        self.alloc.set_i32(EDIT_PAINT_OFFSET, value);
    }
}

/// A list-box column value (`UiListColumn`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiListColumn {
    /// Column position.
    pub position: i32,
    /// Column width.
    pub width: i32,
    /// Maximum characters.
    pub max_chars: i32,
}

/// A list-box definition view (`UiListBoxDefinition`).
#[derive(Debug, Clone)]
pub struct UiListBoxDefinition {
    alloc: UiMemoryAllocation,
}

impl UiListBoxDefinition {
    /// View a list-box record.
    fn new(alloc: UiMemoryAllocation) -> UiListBoxDefinition {
        UiListBoxDefinition { alloc }
    }

    /// Start position.
    #[must_use]
    pub fn start_position(&self) -> i32 {
        self.alloc.get_i32(LIST_START)
    }

    /// Write the start position.
    pub fn set_start_position(&self, value: i32) {
        self.alloc.set_i32(LIST_START, value);
    }

    /// End position.
    #[must_use]
    pub fn end_position(&self) -> i32 {
        self.alloc.get_i32(LIST_END)
    }

    /// Write the end position.
    pub fn set_end_position(&self, value: i32) {
        self.alloc.set_i32(LIST_END, value);
    }

    /// Draw padding.
    #[must_use]
    pub fn draw_padding(&self) -> i32 {
        self.alloc.get_i32(LIST_DRAW_PADDING)
    }

    /// Write the draw padding.
    pub fn set_draw_padding(&self, value: i32) {
        self.alloc.set_i32(LIST_DRAW_PADDING, value);
    }

    /// Cursor position.
    #[must_use]
    pub fn cursor_position(&self) -> i32 {
        self.alloc.get_i32(LIST_CURSOR)
    }

    /// Write the cursor position.
    pub fn set_cursor_position(&self, value: i32) {
        self.alloc.set_i32(LIST_CURSOR, value);
    }

    /// Element width.
    #[must_use]
    pub fn element_width(&self) -> f32 {
        self.alloc.get_f32(LIST_ELEMENT_WIDTH)
    }

    /// Write the element width.
    pub(crate) fn set_element_width(&self, value: f32) {
        self.alloc.set_f32(LIST_ELEMENT_WIDTH, value);
    }

    /// Element height.
    #[must_use]
    pub fn element_height(&self) -> f32 {
        self.alloc.get_f32(LIST_ELEMENT_HEIGHT)
    }

    /// Write the element height.
    pub(crate) fn set_element_height(&self, value: f32) {
        self.alloc.set_f32(LIST_ELEMENT_HEIGHT, value);
    }

    /// Element style.
    #[must_use]
    pub fn element_style(&self) -> i32 {
        self.alloc.get_i32(LIST_ELEMENT_STYLE)
    }

    /// Write the element style.
    pub(crate) fn set_element_style(&self, value: i32) {
        self.alloc.set_i32(LIST_ELEMENT_STYLE, value);
    }

    /// Column count.
    pub(crate) fn column_count(&self) -> i32 {
        self.alloc.get_i32(LIST_COLUMN_COUNT)
    }

    /// Write the column count.
    pub(crate) fn set_column_count(&self, value: i32) {
        self.alloc.set_i32(LIST_COLUMN_COUNT, value);
    }

    /// Column values (donor `columns`).
    pub fn columns(&self) -> Result<Vec<UiListColumn>, ClientError> {
        let count = self.column_count();
        if count > MAX_UI_LIST_COLUMNS as i32 {
            return Err(ClientError::BadUi(
                "listBoxDef_t columns exceed the source 16-slot array".to_string(),
            ));
        }
        let mut columns = Vec::new();
        for index in 0..count.max(0) as usize {
            let offset = LIST_COLUMNS + index * LIST_COLUMN_STRIDE;
            columns.push(UiListColumn {
                position: self.alloc.get_i32(offset),
                width: self.alloc.get_i32(offset + 4),
                max_chars: self.alloc.get_i32(offset + 8),
            });
        }
        Ok(columns)
    }

    /// Column at `index`, or `None` when out of range (donor indexing).
    pub(crate) fn column_at(&self, index: usize) -> Option<UiListColumn> {
        if index >= MAX_UI_LIST_COLUMNS {
            return None;
        }
        let offset = LIST_COLUMNS + index * LIST_COLUMN_STRIDE;
        Some(UiListColumn {
            position: self.alloc.get_i32(offset),
            width: self.alloc.get_i32(offset + 4),
            max_chars: self.alloc.get_i32(offset + 8),
        })
    }

    /// Write one column (donor `setColumn`).
    pub(crate) fn set_column(&self, index: usize, column: &UiListColumn) {
        if index >= MAX_UI_LIST_COLUMNS {
            panic!("listBoxDef_t column index exceeds the source 16-slot array");
        }
        let offset = LIST_COLUMNS + index * LIST_COLUMN_STRIDE;
        self.alloc.set_i32(offset, column.position);
        self.alloc.set_i32(offset + 4, column.width);
        self.alloc.set_i32(offset + 8, column.max_chars);
    }

    /// Double-click script.
    #[must_use]
    pub fn double_click(&self) -> Option<UiScript> {
        self.alloc.get_script(LIST_DOUBLE_CLICK)
    }

    /// Write the double-click script.
    pub(crate) fn set_double_click(&self, value: Option<UiScript>) {
        self.alloc.set_script(LIST_DOUBLE_CLICK, value);
    }

    /// Whether rows are not selectable.
    #[must_use]
    pub fn not_selectable(&self) -> bool {
        self.alloc.get_i32(LIST_NOT_SELECTABLE) != 0
    }

    /// Write the not-selectable flag.
    pub(crate) fn set_not_selectable(&self, value: bool) {
        self.alloc.set_i32(LIST_NOT_SELECTABLE, i32::from(value));
    }
}

/// A multi-choice definition view (`UiMultiDefinition`).
#[derive(Debug, Clone)]
pub struct UiMultiDefinition {
    alloc: UiMemoryAllocation,
}

impl UiMultiDefinition {
    /// View a multi record.
    fn new(alloc: UiMemoryAllocation) -> UiMultiDefinition {
        UiMultiDefinition { alloc }
    }

    /// Choice count.
    #[must_use]
    pub fn count(&self) -> i32 {
        self.alloc.get_i32(MULTI_COUNT)
    }

    /// Write the choice count.
    pub(crate) fn set_count(&self, value: i32) {
        self.alloc.set_i32(MULTI_COUNT, value);
    }

    /// Whether choices carry string values (`cvarStrList`).
    #[must_use]
    pub fn string_definition(&self) -> bool {
        self.alloc.get_i32(MULTI_STRING_DEFINITION) != 0
    }

    /// Write the string-definition flag.
    pub(crate) fn set_string_definition(&self, value: bool) {
        self.alloc.set_i32(MULTI_STRING_DEFINITION, i32::from(value));
    }

    /// Choice label (donor `label`).
    #[must_use]
    pub fn label(&self, index: usize) -> Option<String> {
        self.alloc.get_string(MULTI_LABELS + self.slot(index))
    }

    /// Choice string value (donor `stringValue`).
    #[must_use]
    pub fn string_value(&self, index: usize) -> Option<String> {
        self.alloc.get_string(MULTI_STRING_VALUES + self.slot(index))
    }

    /// Choice number value (donor `numberValue`).
    #[must_use]
    pub fn number_value(&self, index: usize) -> f32 {
        self.alloc.get_f32(MULTI_NUMBER_VALUES + self.slot(index))
    }

    /// Write a choice label.
    pub(crate) fn set_label(&self, index: usize, value: Option<UiStringReference>) {
        let slot = MULTI_LABELS + self.slot(index);
        self.alloc.set_string(slot, value);
    }

    /// Write a choice string value.
    pub(crate) fn set_string_value(&self, index: usize, value: Option<UiStringReference>) {
        let slot = self.slot(index);
        self.alloc.set_string(MULTI_STRING_VALUES + slot, value);
    }

    /// Write a choice number value.
    pub(crate) fn set_number_value(&self, index: usize, value: f32) {
        let slot = self.slot(index);
        self.alloc.set_f32(MULTI_NUMBER_VALUES + slot, value);
    }

    /// Slot offset with donor bounds checking.
    fn slot(&self, index: usize) -> usize {
        if index >= MULTI_SLOTS {
            panic!("multiDef_t index exceeds the source 32-slot arrays");
        }
        index * 4
    }
}

/// A model definition view (`UiModelDefinition`).
#[derive(Debug, Clone)]
pub struct UiModelDefinition {
    alloc: UiMemoryAllocation,
}

impl UiModelDefinition {
    /// View a model record.
    fn new(alloc: UiMemoryAllocation) -> UiModelDefinition {
        UiModelDefinition { alloc }
    }

    /// Model angle.
    #[must_use]
    pub fn angle(&self) -> i32 {
        self.alloc.get_i32(MODEL_ANGLE)
    }

    /// Write the model angle.
    pub fn set_angle(&self, value: i32) {
        self.alloc.set_i32(MODEL_ANGLE, value);
    }

    /// Model origin (live view).
    #[must_use]
    pub fn origin(&self) -> UiMutableVec3 {
        UiMutableVec3::new(&self.alloc, MODEL_ORIGIN)
    }

    /// Horizontal field of view.
    #[must_use]
    pub fn field_of_view_x(&self) -> f32 {
        self.alloc.get_f32(MODEL_FOV_X)
    }

    /// Write the horizontal field of view.
    pub(crate) fn set_field_of_view_x(&self, value: f32) {
        self.alloc.set_f32(MODEL_FOV_X, value);
    }

    /// Vertical field of view.
    #[must_use]
    pub fn field_of_view_y(&self) -> f32 {
        self.alloc.get_f32(MODEL_FOV_Y)
    }

    /// Write the vertical field of view.
    pub(crate) fn set_field_of_view_y(&self, value: f32) {
        self.alloc.set_f32(MODEL_FOV_Y, value);
    }

    /// Rotation speed.
    #[must_use]
    pub fn rotation_speed(&self) -> i32 {
        self.alloc.get_i32(MODEL_ROTATION)
    }

    /// Write the rotation speed.
    pub(crate) fn set_rotation_speed(&self, value: i32) {
        self.alloc.set_i32(MODEL_ROTATION, value);
    }
}

/// Item behavior by type code (`UiItemBehavior`).
#[derive(Debug, Clone)]
pub enum UiItemBehavior {
    /// Static text.
    Text {
        /// Optional edit limits.
        edit: Option<UiEditFieldDefinition>,
    },
    /// Push button.
    Button,
    /// Radio button.
    RadioButton,
    /// Check box.
    CheckBox,
    /// Text edit field.
    EditField {
        /// Edit limits.
        edit: UiEditFieldDefinition,
    },
    /// Combo box.
    Combo,
    /// List box.
    ListBox {
        /// List state.
        list: UiListBoxDefinition,
    },
    /// 3D model.
    Model {
        /// Model data.
        model: Option<UiModelDefinition>,
    },
    /// Engine owner-draw.
    OwnerDraw,
    /// Numeric edit field.
    NumericField {
        /// Edit limits.
        edit: UiEditFieldDefinition,
    },
    /// Slider.
    Slider {
        /// Slider limits.
        edit: UiEditFieldDefinition,
    },
    /// Yes/no toggle.
    YesNo {
        /// Edit limits.
        edit: UiEditFieldDefinition,
    },
    /// Multi-choice.
    Multi {
        /// Choice rows.
        multi: Option<UiMultiDefinition>,
    },
    /// Key binding.
    Bind {
        /// Edit limits.
        edit: UiEditFieldDefinition,
    },
    /// Unknown type code.
    Unknown {
        /// Raw type code.
        type_code: i32,
    },
}

impl UiItemBehavior {
    /// Donor `kind` discriminator.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            UiItemBehavior::Text { .. } => "text",
            UiItemBehavior::Button => "button",
            UiItemBehavior::RadioButton => "radio-button",
            UiItemBehavior::CheckBox => "check-box",
            UiItemBehavior::EditField { .. } => "edit-field",
            UiItemBehavior::Combo => "combo",
            UiItemBehavior::ListBox { .. } => "list-box",
            UiItemBehavior::Model { .. } => "model",
            UiItemBehavior::OwnerDraw => "owner-draw",
            UiItemBehavior::NumericField { .. } => "numeric-field",
            UiItemBehavior::Slider { .. } => "slider",
            UiItemBehavior::YesNo { .. } => "yes-no",
            UiItemBehavior::Multi { .. } => "multi",
            UiItemBehavior::Bind { .. } => "bind",
            UiItemBehavior::Unknown { .. } => "unknown",
        }
    }

    /// Donor `type` code.
    #[must_use]
    pub fn type_code(&self) -> i32 {
        match self {
            UiItemBehavior::Text { .. } => UiItemTypeCode::Text as i32,
            UiItemBehavior::Button => UiItemTypeCode::Button as i32,
            UiItemBehavior::RadioButton => UiItemTypeCode::RadioButton as i32,
            UiItemBehavior::CheckBox => UiItemTypeCode::CheckBox as i32,
            UiItemBehavior::EditField { .. } => UiItemTypeCode::EditField as i32,
            UiItemBehavior::Combo => UiItemTypeCode::Combo as i32,
            UiItemBehavior::ListBox { .. } => UiItemTypeCode::ListBox as i32,
            UiItemBehavior::Model { .. } => UiItemTypeCode::Model as i32,
            UiItemBehavior::OwnerDraw => UiItemTypeCode::OwnerDraw as i32,
            UiItemBehavior::NumericField { .. } => UiItemTypeCode::NumericField as i32,
            UiItemBehavior::Slider { .. } => UiItemTypeCode::Slider as i32,
            UiItemBehavior::YesNo { .. } => UiItemTypeCode::YesNo as i32,
            UiItemBehavior::Multi { .. } => UiItemTypeCode::Multi as i32,
            UiItemBehavior::Bind { .. } => UiItemTypeCode::Bind as i32,
            UiItemBehavior::Unknown { type_code } => *type_code,
        }
    }
}

/// Cvar enable/show rule (`UiCvarRule`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiCvarRule {
    /// Enable when the script matches.
    Enable {
        /// Match script.
        script: Option<UiScript>,
    },
    /// Disable when the script matches.
    Disable {
        /// Match script.
        script: Option<UiScript>,
    },
    /// Show when the script matches.
    Show {
        /// Match script.
        script: Option<UiScript>,
    },
    /// Hide when the script matches.
    Hide {
        /// Match script.
        script: Option<UiScript>,
    },
}

impl UiCvarRule {
    /// Donor `kind` discriminator.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            UiCvarRule::Enable { .. } => "enable",
            UiCvarRule::Disable { .. } => "disable",
            UiCvarRule::Show { .. } => "show",
            UiCvarRule::Hide { .. } => "hide",
        }
    }

    /// Match script.
    #[must_use]
    pub fn script(&self) -> Option<&UiScript> {
        match self {
            UiCvarRule::Enable { script }
            | UiCvarRule::Disable { script }
            | UiCvarRule::Show { script }
            | UiCvarRule::Hide { script } => script.as_ref(),
        }
    }
}
/// A menu item definition view over a 540-byte record (`UiItemDefinition`).
#[derive(Debug, Clone)]
pub struct UiItemDefinition {
    alloc: UiMemoryAllocation,
    location: SourceLocation,
    allocation_offset: Option<usize>,
}

impl UiItemDefinition {
    /// View an item record parsed at `location`.
    fn new(alloc: UiMemoryAllocation, location: SourceLocation, allocation_offset: Option<usize>) -> UiItemDefinition {
        UiItemDefinition {
            alloc,
            location,
            allocation_offset,
        }
    }

    /// Zero the record and apply donor defaults (donor `initialize`).
    pub(crate) fn initialize(&self) {
        self.alloc.clear();
        self.set_text_scale(0.55);
        self.window().initialize();
    }

    /// Parse location of the `itemDef` keyword.
    #[must_use]
    pub fn location(&self) -> &SourceLocation {
        &self.location
    }

    /// Pool offset of the record for owned (`qvm32`) memory.
    #[must_use]
    pub fn allocation_offset(&self) -> Option<usize> {
        self.allocation_offset
    }

    /// Item window (live view).
    #[must_use]
    pub fn window(&self) -> UiWindowDefinition {
        UiWindowDefinition::new(self.alloc.subrecord(ITEM_WINDOW, WINDOW_SIZE))
    }

    /// Raw type code (donor `type`).
    #[must_use]
    pub fn item_type(&self) -> i32 {
        self.alloc.get_i32(ITEM_TYPE)
    }

    /// Write the raw type code.
    pub(crate) fn set_item_type(&self, value: i32) {
        self.alloc.set_i32(ITEM_TYPE, value);
    }

    /// Parent menu definition.
    #[must_use]
    pub fn parent(&self) -> Option<UiMenuDefinition> {
        self.alloc.get_menu(ITEM_PARENT)
    }

    /// Write the parent menu definition.
    pub fn set_parent(&self, value: Option<UiMenuDefinition>) {
        self.alloc.set_menu(ITEM_PARENT, value);
    }

    /// Behavior selected by the type code (donor `behavior`).
    #[must_use]
    pub fn behavior(&self) -> UiItemBehavior {
        match UiItemTypeCode::from_i32(self.item_type()) {
            Some(UiItemTypeCode::Text) => UiItemBehavior::Text { edit: self.edit_data() },
            Some(UiItemTypeCode::Button) => UiItemBehavior::Button,
            Some(UiItemTypeCode::RadioButton) => UiItemBehavior::RadioButton,
            Some(UiItemTypeCode::CheckBox) => UiItemBehavior::CheckBox,
            Some(UiItemTypeCode::EditField) => UiItemBehavior::EditField {
                edit: self.required_edit_data(),
            },
            Some(UiItemTypeCode::Combo) => UiItemBehavior::Combo,
            Some(UiItemTypeCode::ListBox) => UiItemBehavior::ListBox {
                list: match self.list_data() {
                    Some(list) => list,
                    None => panic!("UI list operation dereferences NULL typeData"),
                },
            },
            Some(UiItemTypeCode::Model) => UiItemBehavior::Model {
                model: self.model_data(),
            },
            Some(UiItemTypeCode::OwnerDraw) => UiItemBehavior::OwnerDraw,
            Some(UiItemTypeCode::NumericField) => UiItemBehavior::NumericField {
                edit: self.required_edit_data(),
            },
            Some(UiItemTypeCode::Slider) => UiItemBehavior::Slider {
                edit: self.required_edit_data(),
            },
            Some(UiItemTypeCode::YesNo) => UiItemBehavior::YesNo {
                edit: self.required_edit_data(),
            },
            Some(UiItemTypeCode::Multi) => UiItemBehavior::Multi {
                multi: self.multi_data(),
            },
            Some(UiItemTypeCode::Bind) => UiItemBehavior::Bind {
                edit: self.required_edit_data(),
            },
            None => UiItemBehavior::Unknown {
                type_code: self.item_type(),
            },
        }
    }

    /// Text rectangle (live view).
    #[must_use]
    pub fn text_rect(&self) -> UiMutableRect {
        UiMutableRect::new(&self.alloc, ITEM_TEXT_RECT)
    }

    /// Write the text rectangle.
    pub fn set_text_rect(&self, value: &UiRect) {
        self.text_rect().write(value);
    }

    /// Alignment.
    #[must_use]
    pub fn alignment(&self) -> i32 {
        self.alloc.get_i32(ITEM_ALIGNMENT)
    }

    /// Write the alignment.
    pub(crate) fn set_alignment(&self, value: i32) {
        self.alloc.set_i32(ITEM_ALIGNMENT, value);
    }

    /// Text alignment.
    #[must_use]
    pub fn text_alignment(&self) -> i32 {
        self.alloc.get_i32(ITEM_TEXT_ALIGNMENT)
    }

    /// Write the text alignment.
    pub(crate) fn set_text_alignment(&self, value: i32) {
        self.alloc.set_i32(ITEM_TEXT_ALIGNMENT, value);
    }

    /// Text align x.
    #[must_use]
    pub fn text_align_x(&self) -> f32 {
        self.alloc.get_f32(ITEM_TEXT_ALIGN_X)
    }

    /// Write the text align x.
    pub(crate) fn set_text_align_x(&self, value: f32) {
        self.alloc.set_f32(ITEM_TEXT_ALIGN_X, value);
    }

    /// Text align y.
    #[must_use]
    pub fn text_align_y(&self) -> f32 {
        self.alloc.get_f32(ITEM_TEXT_ALIGN_Y)
    }

    /// Write the text align y.
    pub(crate) fn set_text_align_y(&self, value: f32) {
        self.alloc.set_f32(ITEM_TEXT_ALIGN_Y, value);
    }

    /// Text scale.
    #[must_use]
    pub fn text_scale(&self) -> f32 {
        self.alloc.get_f32(ITEM_TEXT_SCALE)
    }

    /// Write the text scale.
    pub(crate) fn set_text_scale(&self, value: f32) {
        self.alloc.set_f32(ITEM_TEXT_SCALE, value);
    }

    /// Text style.
    #[must_use]
    pub fn text_style(&self) -> i32 {
        self.alloc.get_i32(ITEM_TEXT_STYLE)
    }

    /// Write the text style.
    pub(crate) fn set_text_style(&self, value: i32) {
        self.alloc.set_i32(ITEM_TEXT_STYLE, value);
    }

    /// Item text.
    #[must_use]
    pub fn text(&self) -> Option<String> {
        self.alloc.get_string(ITEM_TEXT)
    }

    /// Write the item text.
    pub(crate) fn set_text(&self, value: Option<UiStringReference>) {
        self.alloc.set_string(ITEM_TEXT, value);
    }

    /// Shader or model asset.
    #[must_use]
    pub fn asset(&self) -> Option<UiMenuResource> {
        match self.alloc.get_resource(ITEM_ASSET) {
            None => None,
            Some(UiMenuResource::Shader(value)) => Some(UiMenuResource::Shader(value)),
            Some(UiMenuResource::Model(value)) => Some(UiMenuResource::Model(value)),
            Some(_) => panic!("UI item asset aliases a sound handle"),
        }
    }

    /// Asset registration handle.
    #[must_use]
    pub fn asset_handle(&self) -> Option<i32> {
        self.alloc.get_resource_handle(ITEM_ASSET)
    }

    /// Write the asset plus handle.
    pub(crate) fn set_asset(&self, value: UiMenuResource, handle: Option<i32>) {
        self.alloc.set_resource(ITEM_ASSET, value, handle);
    }

    /// Mouse-enter text script.
    #[must_use]
    pub fn mouse_enter_text(&self) -> Option<UiScript> {
        self.alloc.get_script(ITEM_MOUSE_ENTER_TEXT)
    }

    /// Write the mouse-enter text script.
    pub(crate) fn set_mouse_enter_text(&self, value: Option<UiScript>) {
        self.alloc.set_script(ITEM_MOUSE_ENTER_TEXT, value);
    }

    /// Mouse-exit text script.
    #[must_use]
    pub fn mouse_exit_text(&self) -> Option<UiScript> {
        self.alloc.get_script(ITEM_MOUSE_EXIT_TEXT)
    }

    /// Write the mouse-exit text script.
    pub(crate) fn set_mouse_exit_text(&self, value: Option<UiScript>) {
        self.alloc.set_script(ITEM_MOUSE_EXIT_TEXT, value);
    }

    /// Mouse-enter script.
    #[must_use]
    pub fn mouse_enter(&self) -> Option<UiScript> {
        self.alloc.get_script(ITEM_MOUSE_ENTER)
    }

    /// Write the mouse-enter script.
    pub(crate) fn set_mouse_enter(&self, value: Option<UiScript>) {
        self.alloc.set_script(ITEM_MOUSE_ENTER, value);
    }

    /// Mouse-exit script.
    #[must_use]
    pub fn mouse_exit(&self) -> Option<UiScript> {
        self.alloc.get_script(ITEM_MOUSE_EXIT)
    }

    /// Write the mouse-exit script.
    pub(crate) fn set_mouse_exit(&self, value: Option<UiScript>) {
        self.alloc.set_script(ITEM_MOUSE_EXIT, value);
    }

    /// Action script.
    #[must_use]
    pub fn action(&self) -> Option<UiScript> {
        self.alloc.get_script(ITEM_ACTION)
    }

    /// Write the action script.
    pub(crate) fn set_action(&self, value: Option<UiScript>) {
        self.alloc.set_script(ITEM_ACTION, value);
    }

    /// Focus script.
    #[must_use]
    pub fn on_focus(&self) -> Option<UiScript> {
        self.alloc.get_script(ITEM_ON_FOCUS)
    }

    /// Write the focus script.
    pub(crate) fn set_on_focus(&self, value: Option<UiScript>) {
        self.alloc.set_script(ITEM_ON_FOCUS, value);
    }

    /// Leave-focus script.
    #[must_use]
    pub fn leave_focus(&self) -> Option<UiScript> {
        self.alloc.get_script(ITEM_LEAVE_FOCUS)
    }

    /// Write the leave-focus script.
    pub(crate) fn set_leave_focus(&self, value: Option<UiScript>) {
        self.alloc.set_script(ITEM_LEAVE_FOCUS, value);
    }

    /// Bound cvar name.
    #[must_use]
    pub fn cvar(&self) -> Option<String> {
        self.alloc.get_string(ITEM_CVAR)
    }

    /// Write the bound cvar name.
    pub(crate) fn set_cvar(&self, value: Option<UiStringReference>) {
        self.alloc.set_string(ITEM_CVAR, value);
    }

    /// Cvar test string.
    #[must_use]
    pub fn cvar_test(&self) -> Option<String> {
        self.alloc.get_string(ITEM_CVAR_TEST)
    }

    /// Write the cvar test string.
    pub(crate) fn set_cvar_test(&self, value: Option<UiStringReference>) {
        self.alloc.set_string(ITEM_CVAR_TEST, value);
    }

    /// Cvar rule decoded from flags plus script (donor `cvarRule`).
    #[must_use]
    pub fn cvar_rule(&self) -> Option<UiCvarRule> {
        let flags = self.cvar_flags();
        if flags & CVAR_RULE_MASK == 0 {
            return None;
        }
        let script = self.cvar_script();
        if flags & CVAR_RULE_ENABLE != 0 {
            Some(UiCvarRule::Enable { script })
        } else if flags & CVAR_RULE_DISABLE != 0 {
            Some(UiCvarRule::Disable { script })
        } else if flags & CVAR_RULE_SHOW != 0 {
            Some(UiCvarRule::Show { script })
        } else if flags & CVAR_RULE_HIDE != 0 {
            Some(UiCvarRule::Hide { script })
        } else {
            None
        }
    }

    /// Write the cvar rule script plus flag bit (donor `cvarRule` setter).
    pub(crate) fn set_cvar_rule(&self, value: Option<UiCvarRule>) {
        let (script, flags) = match value {
            None => (None, 0),
            Some(UiCvarRule::Enable { script }) => (script, CVAR_RULE_ENABLE),
            Some(UiCvarRule::Disable { script }) => (script, CVAR_RULE_DISABLE),
            Some(UiCvarRule::Show { script }) => (script, CVAR_RULE_SHOW),
            Some(UiCvarRule::Hide { script }) => (script, CVAR_RULE_HIDE),
        };
        self.alloc.set_script(ITEM_CVAR_SCRIPT, script);
        self.alloc.set_i32(ITEM_CVAR_FLAGS, flags);
    }

    /// Raw cvar flags.
    #[must_use]
    pub fn cvar_flags(&self) -> i32 {
        self.alloc.get_i32(ITEM_CVAR_FLAGS)
    }

    /// Cvar rule script.
    #[must_use]
    pub fn cvar_script(&self) -> Option<UiScript> {
        self.alloc.get_script(ITEM_CVAR_SCRIPT)
    }

    /// Focus sound reference.
    #[must_use]
    pub fn focus_sound(&self) -> Option<UiSoundReference> {
        match self.alloc.get_resource(ITEM_FOCUS_SOUND) {
            None => None,
            Some(UiMenuResource::Sound(value)) => Some(value),
            Some(_) => panic!("UI focus sound aliases a different resource kind"),
        }
    }

    /// Focus sound handle.
    #[must_use]
    pub fn focus_sound_handle(&self) -> Option<i32> {
        self.alloc.get_resource_handle(ITEM_FOCUS_SOUND)
    }

    /// Write the focus sound plus handle.
    pub(crate) fn set_focus_sound(&self, value: UiSoundReference, handle: Option<i32>) {
        self.alloc
            .set_resource(ITEM_FOCUS_SOUND, UiMenuResource::Sound(value), handle);
    }

    /// Color range count.
    pub(crate) fn color_count(&self) -> i32 {
        self.alloc.get_i32(ITEM_COLOR_COUNT)
    }

    /// Write the color range count.
    pub(crate) fn set_color_count(&self, value: i32) {
        self.alloc.set_i32(ITEM_COLOR_COUNT, value);
    }

    /// Color range values (donor `colorRanges`).
    pub fn color_ranges(&self) -> Result<Vec<UiColorRange>, ClientError> {
        let count = self.color_count();
        if count > MAX_UI_COLOR_RANGES as i32 {
            return Err(ClientError::BadUi(
                "UI color ranges exceed the source array".to_string(),
            ));
        }
        let mut ranges = Vec::new();
        for index in 0..count.max(0) as usize {
            let offset = ITEM_COLOR_RANGES + index * ITEM_COLOR_STRIDE;
            ranges.push(UiColorRange {
                low: self.alloc.get_f32(offset + 16),
                high: self.alloc.get_f32(offset + 20),
                color: UiMutableColor::new(&self.alloc, offset).snapshot(),
            });
        }
        Ok(ranges)
    }

    /// Append a color range unless the array is full (donor `addColorRange`).
    pub(crate) fn add_color_range(&self, value: &UiColorRange) {
        let index = self.color_count();
        if index >= MAX_UI_COLOR_RANGES as i32 {
            return;
        }
        if index < 0 {
            panic!("UI color range index is negative");
        }
        let offset = ITEM_COLOR_RANGES + index as usize * ITEM_COLOR_STRIDE;
        UiMutableColor::new(&self.alloc, offset).write(&value.color);
        self.alloc.set_f32(offset + 16, value.low);
        self.alloc.set_f32(offset + 20, value.high);
        self.set_color_count(index + 1);
    }

    /// Feeder id (`special`).
    #[must_use]
    pub fn special(&self) -> f32 {
        self.alloc.get_f32(ITEM_SPECIAL)
    }

    /// Write the feeder id.
    pub fn set_special(&self, value: f32) {
        self.alloc.set_f32(ITEM_SPECIAL, value);
    }

    /// Cursor position.
    #[must_use]
    pub fn cursor_position(&self) -> i32 {
        self.alloc.get_i32(ITEM_CURSOR)
    }

    /// Write the cursor position.
    pub fn set_cursor_position(&self, value: i32) {
        self.alloc.set_i32(ITEM_CURSOR, value);
    }

    /// Type-data pointer (donor `typeData`).
    pub(crate) fn type_data(&self) -> Option<UiMemoryAllocation> {
        self.alloc.get_allocation_pointer(ITEM_TYPE_DATA)
    }

    /// Write the type-data pointer.
    pub(crate) fn set_type_data(&self, value: Option<UiMemoryAllocation>) {
        self.alloc.set_allocation_pointer(ITEM_TYPE_DATA, value);
    }

    /// Whether a type-data pointer is present (donor `hasTypeData`).
    pub(crate) fn has_type_data(&self) -> bool {
        !self.alloc.is_null_pointer(ITEM_TYPE_DATA)
    }

    /// Edit-field type data, if any (donor `editData`).
    #[must_use]
    pub fn edit_data(&self) -> Option<UiEditFieldDefinition> {
        self.type_data()
            .map(|data| UiEditFieldDefinition::new(data.dereference(EDIT_SIZE)))
    }

    /// List-box type data, if any (donor `listData`).
    #[must_use]
    pub fn list_data(&self) -> Option<UiListBoxDefinition> {
        self.type_data()
            .map(|data| UiListBoxDefinition::new(data.dereference(LIST_SIZE)))
    }

    /// Model type data, if any (donor `modelData`).
    #[must_use]
    pub fn model_data(&self) -> Option<UiModelDefinition> {
        self.type_data()
            .map(|data| UiModelDefinition::new(data.dereference(MODEL_SIZE)))
    }

    /// Multi type data, if any (donor `multiData`).
    #[must_use]
    pub fn multi_data(&self) -> Option<UiMultiDefinition> {
        self.type_data()
            .map(|data| UiMultiDefinition::new(data.dereference(MULTI_SIZE)))
    }

    /// Required edit-field type data (donor `requiredEditData`).
    fn required_edit_data(&self) -> UiEditFieldDefinition {
        match self.edit_data() {
            Some(edit) => edit,
            None => panic!("UI edit operation dereferences NULL typeData"),
        }
    }
}

/// A menu definition view over a 644-byte record (`UiMenuDefinition`).
#[derive(Debug, Clone)]
pub struct UiMenuDefinition {
    alloc: UiMemoryAllocation,
    location: SourceLocation,
}

impl UiMenuDefinition {
    /// View a menu record parsed at `location`.
    fn new(alloc: UiMemoryAllocation, location: SourceLocation) -> UiMenuDefinition {
        UiMenuDefinition { alloc, location }
    }

    /// Zero the record and inherit asset fades (donor `initialize`).
    pub(crate) fn initialize(&self, assets: &UiGlobalAssets) {
        self.alloc.clear();
        self.set_cursor_item(-1);
        self.set_fade_amount(assets.fade_amount);
        self.set_fade_clamp(assets.fade_clamp);
        self.set_fade_cycle(assets.fade_cycle);
        self.window().initialize();
    }

    /// Parse location of the `menuDef` keyword.
    #[must_use]
    pub fn location(&self) -> &SourceLocation {
        &self.location
    }

    /// Whether two views alias the same menu record (donor `===` on menus).
    pub(crate) fn same_record(&self, other: &Self) -> bool {
        self.alloc.same_span(&other.alloc)
    }

    /// Static menu slot (donor `sourceIndex`).
    #[must_use]
    pub fn source_index(&self) -> usize {
        self.alloc.offset() / MENU_SIZE
    }

    /// Menu window (live view).
    #[must_use]
    pub fn window(&self) -> UiWindowDefinition {
        UiWindowDefinition::new(self.alloc.subrecord(MENU_WINDOW, WINDOW_SIZE))
    }

    /// Menu font name.
    #[must_use]
    pub fn font(&self) -> Option<String> {
        self.alloc.get_string(MENU_FONT)
    }

    /// Write the menu font name.
    pub(crate) fn set_font(&self, value: Option<UiStringReference>) {
        self.alloc.set_string(MENU_FONT, value);
    }

    /// Full-screen flag.
    #[must_use]
    pub fn full_screen(&self) -> i32 {
        self.alloc.get_i32(MENU_FULL_SCREEN)
    }

    /// Write the full-screen flag.
    pub(crate) fn set_full_screen(&self, value: i32) {
        self.alloc.set_i32(MENU_FULL_SCREEN, value);
    }

    /// Item count.
    #[must_use]
    pub fn item_count(&self) -> i32 {
        self.alloc.get_i32(MENU_ITEM_COUNT)
    }

    /// Write the item count.
    pub(crate) fn set_item_count(&self, value: i32) {
        self.alloc.set_i32(MENU_ITEM_COUNT, value);
    }

    /// Font index (never written by the parser; always zero).
    #[must_use]
    pub fn font_index(&self) -> i32 {
        self.alloc.get_i32(MENU_FONT_INDEX)
    }

    /// Cursor item index.
    #[must_use]
    pub fn cursor_item(&self) -> i32 {
        self.alloc.get_i32(MENU_CURSOR_ITEM)
    }

    /// Write the cursor item index.
    pub fn set_cursor_item(&self, value: i32) {
        self.alloc.set_i32(MENU_CURSOR_ITEM, value);
    }

    /// Fade cycle.
    #[must_use]
    pub fn fade_cycle(&self) -> i32 {
        self.alloc.get_i32(MENU_FADE_CYCLE)
    }

    /// Write the fade cycle.
    pub(crate) fn set_fade_cycle(&self, value: i32) {
        self.alloc.set_i32(MENU_FADE_CYCLE, value);
    }

    /// Fade clamp.
    #[must_use]
    pub fn fade_clamp(&self) -> f32 {
        self.alloc.get_f32(MENU_FADE_CLAMP)
    }

    /// Write the fade clamp.
    pub(crate) fn set_fade_clamp(&self, value: f32) {
        self.alloc.set_f32(MENU_FADE_CLAMP, value);
    }

    /// Fade amount.
    #[must_use]
    pub fn fade_amount(&self) -> f32 {
        self.alloc.get_f32(MENU_FADE_AMOUNT)
    }

    /// Write the fade amount.
    pub(crate) fn set_fade_amount(&self, value: f32) {
        self.alloc.set_f32(MENU_FADE_AMOUNT, value);
    }

    /// Open script.
    #[must_use]
    pub fn on_open(&self) -> Option<UiScript> {
        self.alloc.get_script(MENU_ON_OPEN)
    }

    /// Write the open script.
    pub(crate) fn set_on_open(&self, value: Option<UiScript>) {
        self.alloc.set_script(MENU_ON_OPEN, value);
    }

    /// Close script.
    #[must_use]
    pub fn on_close(&self) -> Option<UiScript> {
        self.alloc.get_script(MENU_ON_CLOSE)
    }

    /// Write the close script.
    pub(crate) fn set_on_close(&self, value: Option<UiScript>) {
        self.alloc.set_script(MENU_ON_CLOSE, value);
    }

    /// Escape script.
    #[must_use]
    pub fn on_escape(&self) -> Option<UiScript> {
        self.alloc.get_script(MENU_ON_ESCAPE)
    }

    /// Write the escape script.
    pub(crate) fn set_on_escape(&self, value: Option<UiScript>) {
        self.alloc.set_script(MENU_ON_ESCAPE, value);
    }

    /// Looped menu sound.
    #[must_use]
    pub fn sound_loop(&self) -> Option<UiSoundReference> {
        self.alloc
            .get_string(MENU_SOUND_LOOP)
            .map(|path| UiSoundReference { path: Some(path) })
    }

    /// Write the looped menu sound path.
    pub(crate) fn set_sound_loop(&self, value: Option<UiStringReference>) {
        self.alloc.set_string(MENU_SOUND_LOOP, value);
    }

    /// Focus color (live view).
    #[must_use]
    pub fn focus_color(&self) -> UiMutableColor {
        UiMutableColor::new(&self.alloc, MENU_FOCUS_COLOR)
    }

    /// Disable color (live view).
    #[must_use]
    pub fn disable_color(&self) -> UiMutableColor {
        UiMutableColor::new(&self.alloc, MENU_DISABLE_COLOR)
    }

    /// Write one item slot (donor `setItem`).
    pub(crate) fn set_item(&self, index: usize, item: Option<UiItemDefinition>) {
        if index >= MAX_UI_MENU_ITEMS {
            panic!("UI menu item slot exceeds the source array");
        }
        self.alloc.set_item(MENU_ITEMS + index * 4, item);
    }

    /// Read one item slot, or `None` for a null pointer.
    pub(crate) fn slot_item(&self, index: usize) -> Option<UiItemDefinition> {
        if index >= MAX_UI_MENU_ITEMS {
            panic!("UI menu item slot exceeds the source array");
        }
        self.alloc.get_item(MENU_ITEMS + index * 4)
    }

    /// Item at `index`, or `None` for a null slot (donor `itemAt`).
    #[must_use]
    pub fn item_at(&self, index: usize) -> Option<UiItemDefinition> {
        self.slot_item(index)
    }

    /// Item view at `index` (donor indexing; out-of-range and null slots error).
    pub(crate) fn item_view(&self, index: usize) -> Result<UiItemDefinition, ClientError> {
        if index >= MAX_UI_MENU_ITEMS {
            return Err(ClientError::BadUi(
                "UI menu item index is out of range".to_string(),
            ));
        }
        self.slot_item(index).ok_or_else(|| {
            ClientError::BadUi("UI menu dereferences a NULL item pointer".to_string())
        })
    }

    /// Retained items, in slot order (donor `items`).
    pub fn items(&self) -> Result<Vec<UiItemDefinition>, ClientError> {
        let count = self.item_count();
        if count > MAX_UI_MENU_ITEMS as i32 {
            return Err(ClientError::BadUi(
                "UI menu item count exceeds the source array".to_string(),
            ));
        }
        let mut items = Vec::new();
        for index in 0..count.max(0) as usize {
            match self.slot_item(index) {
                Some(item) => items.push(item),
                None => {
                    return Err(ClientError::BadUi(
                        "UI menu dereferences a NULL item pointer".to_string(),
                    ))
                }
            }
        }
        Ok(items)
    }
}

/// Global menu assets (`UiGlobalAssets`).
#[derive(Debug, Clone, PartialEq)]
pub struct UiGlobalAssets {
    /// Text font.
    pub text_font: Option<UiFontReference>,
    /// Small font.
    pub small_font: Option<UiFontReference>,
    /// Big font.
    pub big_font: Option<UiFontReference>,
    /// Cursor shader.
    pub cursor: Option<UiShaderReference>,
    /// Gradient bar shader.
    pub gradient_bar: Option<UiShaderReference>,
    /// Menu enter sound.
    pub menu_enter_sound: Option<UiSoundReference>,
    /// Menu exit sound.
    pub menu_exit_sound: Option<UiSoundReference>,
    /// Menu buzz sound.
    pub menu_buzz_sound: Option<UiSoundReference>,
    /// Item focus sound.
    pub item_focus_sound: Option<UiSoundReference>,
    /// Fade clamp.
    pub fade_clamp: f32,
    /// Fade cycle.
    pub fade_cycle: i32,
    /// Fade amount.
    pub fade_amount: f32,
    /// Shadow x offset.
    pub shadow_x: f32,
    /// Shadow y offset.
    pub shadow_y: f32,
    /// Shadow color.
    pub shadow_color: Vec4,
    /// Shadow fade clamp.
    pub shadow_fade_clamp: f32,
}

impl Default for UiGlobalAssets {
    /// Empty assets (donor `newAssets(undefined)`).
    fn default() -> UiGlobalAssets {
        UiGlobalAssets {
            text_font: None,
            small_font: None,
            big_font: None,
            cursor: None,
            gradient_bar: None,
            menu_enter_sound: None,
            menu_exit_sound: None,
            menu_buzz_sound: None,
            item_focus_sound: None,
            fade_clamp: 0.0,
            fade_cycle: 0,
            fade_amount: 0.0,
            shadow_x: 0.0,
            shadow_y: 0.0,
            shadow_color: Vec4 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 0.0,
            },
            shadow_fade_clamp: 0.0,
        }
    }
}

/// A registration request (`UiMenuRegistrationEvent`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiMenuRegistrationEvent {
    /// Register a font.
    Font {
        /// Font reference.
        reference: UiFontReference,
        /// Request location.
        location: SourceLocation,
    },
    /// Register a picture.
    Picture {
        /// Shader reference.
        reference: UiShaderReference,
        /// Request location.
        location: SourceLocation,
    },
    /// Register a sound.
    Sound {
        /// Sound reference.
        reference: UiSoundReference,
        /// Request location.
        location: SourceLocation,
    },
    /// Register a model.
    Model {
        /// Model reference.
        reference: UiModelReference,
        /// Request location.
        location: SourceLocation,
    },
}

impl UiMenuRegistrationEvent {
    /// Donor `kind` discriminator.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            UiMenuRegistrationEvent::Font { .. } => "font",
            UiMenuRegistrationEvent::Picture { .. } => "picture",
            UiMenuRegistrationEvent::Sound { .. } => "sound",
            UiMenuRegistrationEvent::Model { .. } => "model",
        }
    }

    /// Request location.
    #[must_use]
    pub fn location(&self) -> &SourceLocation {
        match self {
            UiMenuRegistrationEvent::Font { location, .. }
            | UiMenuRegistrationEvent::Picture { location, .. }
            | UiMenuRegistrationEvent::Sound { location, .. }
            | UiMenuRegistrationEvent::Model { location, .. } => location,
        }
    }
}

/// A registration handle (`{ handle }` in the donor).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiMenuRegistrationHandle {
    /// Registered handle.
    pub handle: i32,
}

/// A registration outcome: `None` for `void` (`UiMenuRegistrationResult`).
pub type UiMenuRegistrationResult = Option<UiMenuRegistrationHandle>;

/// Sync registration sink (`UiMenuRegistrationSink`; donor `Promise -> sync`).
pub trait UiMenuRegistrationSink {
    /// Register one asset, optionally returning a handle.
    fn register(&mut self, event: &UiMenuRegistrationEvent) -> Result<UiMenuRegistrationResult, ClientError>;
}

/// An asset publication (`UiMenuAssetPublication`).
#[derive(Debug, Clone, PartialEq)]
pub enum UiMenuAssetPublication {
    /// Text font changed.
    TextFont(UiFontReference),
    /// Small font changed.
    SmallFont(UiFontReference),
    /// Big font changed.
    BigFont(UiFontReference),
    /// Cursor shader changed.
    Cursor(UiShaderReference),
    /// Gradient bar changed.
    GradientBar(UiShaderReference),
    /// Menu enter sound changed.
    MenuEnterSound(UiSoundReference),
    /// Menu exit sound changed.
    MenuExitSound(UiSoundReference),
    /// Menu buzz sound changed.
    MenuBuzzSound(UiSoundReference),
    /// Item focus sound changed.
    ItemFocusSound(UiSoundReference),
    /// Fade clamp changed.
    FadeClamp(f32),
    /// Fade cycle changed.
    FadeCycle(i32),
    /// Fade amount changed.
    FadeAmount(f32),
    /// Shadow x changed.
    ShadowX(f32),
    /// Shadow y changed.
    ShadowY(f32),
    /// Shadow fade clamp changed.
    ShadowFadeClamp(f32),
    /// One shadow color component changed.
    ShadowColorComponent {
        /// Component selector.
        component: UiColorComponent,
        /// New value.
        value: f32,
    },
    /// Font-registered flag changed.
    FontRegistered(bool),
    /// Raw cursor string pointer changed.
    CursorStr(Option<UiStringReference>),
}

impl UiMenuAssetPublication {
    /// Donor `field` discriminator.
    #[must_use]
    pub fn field(&self) -> &'static str {
        match self {
            UiMenuAssetPublication::TextFont(_) => "textFont",
            UiMenuAssetPublication::SmallFont(_) => "smallFont",
            UiMenuAssetPublication::BigFont(_) => "bigFont",
            UiMenuAssetPublication::Cursor(_) => "cursor",
            UiMenuAssetPublication::GradientBar(_) => "gradientBar",
            UiMenuAssetPublication::MenuEnterSound(_) => "menuEnterSound",
            UiMenuAssetPublication::MenuExitSound(_) => "menuExitSound",
            UiMenuAssetPublication::MenuBuzzSound(_) => "menuBuzzSound",
            UiMenuAssetPublication::ItemFocusSound(_) => "itemFocusSound",
            UiMenuAssetPublication::FadeClamp(_) => "fadeClamp",
            UiMenuAssetPublication::FadeCycle(_) => "fadeCycle",
            UiMenuAssetPublication::FadeAmount(_) => "fadeAmount",
            UiMenuAssetPublication::ShadowX(_) => "shadowX",
            UiMenuAssetPublication::ShadowY(_) => "shadowY",
            UiMenuAssetPublication::ShadowFadeClamp(_) => "shadowFadeClamp",
            UiMenuAssetPublication::ShadowColorComponent { .. } => "shadowColorComponent",
            UiMenuAssetPublication::FontRegistered(_) => "fontRegistered",
            UiMenuAssetPublication::CursorStr(_) => "cursorStr",
        }
    }
}

/// Sync asset sink (`UiMenuAssetSink`).
pub trait UiMenuAssetSink {
    /// Publish one asset change.
    fn publish(&mut self, event: &UiMenuAssetPublication);
}

/// Registration completion state (`UiMenuRegistrationState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiMenuRegistrationState {
    /// No registration sink ran; events are retained for later.
    Deferred {
        /// Retained events.
        events: Vec<UiMenuRegistrationEvent>,
    },
    /// A registration sink ran over these events.
    Completed {
        /// Retained events.
        events: Vec<UiMenuRegistrationEvent>,
    },
}

impl UiMenuRegistrationState {
    /// Donor `kind` discriminator.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            UiMenuRegistrationState::Deferred { .. } => "deferred",
            UiMenuRegistrationState::Completed { .. } => "completed",
        }
    }

    /// Retained events.
    #[must_use]
    pub fn events(&self) -> &[UiMenuRegistrationEvent] {
        match self {
            UiMenuRegistrationState::Deferred { events } | UiMenuRegistrationState::Completed { events } => events,
        }
    }
}

/// Parsed menu definitions (`UiMenuDefinitions`).
#[derive(Debug, Clone)]
pub struct UiMenuDefinitions {
    /// Memory ownership echoed from the parse options.
    pub memory: UiMenuMemoryOwnership,
    /// Parsed menus, in load order.
    pub menus: Vec<UiMenuDefinition>,
    /// Final global assets.
    pub assets: UiGlobalAssets,
    /// Loaded file paths, in load order.
    pub loaded_files: Vec<String>,
    /// Recorded diagnostics.
    pub diagnostics: Vec<ScriptDiagnostic>,
    /// Registration state.
    pub registration: UiMenuRegistrationState,
    /// Whether a menu font was registered outside HUD parsing.
    pub font_registered: bool,
}

/// Menu include resolver with root lookup (`UiMenuResolver`).
pub trait UiMenuResolver: IncludeResolver {
    /// Resolve a root menu path, or `None` when the file is missing.
    fn resolve_root(&mut self, path: &str) -> Option<ScriptSource>;
}

/// Shared menu resolver handle (one owner across many token cursors).
///
/// The donor passes one resolver object to every cursor; the sibling reader
/// takes resolvers by value, so cursors share this handle instead.
#[derive(Clone)]
pub struct SharedUiMenuResolver {
    inner: Rc<RefCell<Box<dyn UiMenuResolver>>>,
}

impl SharedUiMenuResolver {
    /// Share any resolver.
    pub fn new(resolver: impl UiMenuResolver + 'static) -> SharedUiMenuResolver {
        SharedUiMenuResolver {
            inner: Rc::new(RefCell::new(Box::new(resolver))),
        }
    }

    /// Resolve a root menu path through the shared resolver.
    pub fn resolve_root(&self, path: &str) -> Option<ScriptSource> {
        self.inner.borrow_mut().resolve_root(path)
    }
}

impl IncludeResolver for SharedUiMenuResolver {
    /// Resolve an include through the shared resolver.
    fn resolve(&mut self, request: &IncludeRequest) -> Result<Option<ScriptSource>, ClientError> {
        self.inner.borrow_mut().resolve(request)
    }
}

/// QVM `rand()` in `0..=32767` (`UiMenuRandom`).
pub trait UiMenuRandom {
    /// Next random integer.
    fn next_int(&mut self) -> i32;
}

/// Handle-based script sources (donor `UiMenuScriptSources`).
pub trait UiMenuScriptSources {
    /// Load a source by path, returning `0` when missing.
    fn load_source_handle(&mut self, path: &str) -> i32;
    /// Read the next token record, if any.
    fn read_token_handle(&mut self, handle: i32) -> Option<ScriptTokenRecord>;
    /// Current file and line for a handle.
    fn source_file_and_line(&mut self, handle: i32) -> Option<ScriptSourcePosition>;
    /// Free a source handle.
    fn free_source_handle(&mut self, handle: i32) -> bool;
}

/// Shared handle-based sources.
#[derive(Clone)]
pub struct SharedUiMenuScriptSources {
    inner: Rc<RefCell<Box<dyn UiMenuScriptSources>>>,
}

impl SharedUiMenuScriptSources {
    /// Share any handle-based sources.
    pub fn new(sources: impl UiMenuScriptSources + 'static) -> SharedUiMenuScriptSources {
        SharedUiMenuScriptSources {
            inner: Rc::new(RefCell::new(Box::new(sources))),
        }
    }

    /// Load a source by path through the shared sources.
    pub fn load_source_handle(&self, path: &str) -> i32 {
        self.inner.borrow_mut().load_source_handle(path)
    }

    /// Read the next token record through the shared sources.
    pub fn read_token_handle(&self, handle: i32) -> Option<ScriptTokenRecord> {
        self.inner.borrow_mut().read_token_handle(handle)
    }

    /// Current file and line through the shared sources.
    pub fn source_file_and_line(&self, handle: i32) -> Option<ScriptSourcePosition> {
        self.inner.borrow_mut().source_file_and_line(handle)
    }

    /// Free a handle through the shared sources.
    pub fn free_source_handle(&self, handle: i32) -> bool {
        self.inner.borrow_mut().free_source_handle(handle)
    }
}

/// Current-operation assertion for handle hosts (donor `assertCurrentOperation`).
pub type UiMenuOperationGuard = Rc<dyn Fn()>;

/// Resolver-backed parse host (`UiMenuResolvedParseHost`).
pub struct UiMenuResolvedParseHost {
    /// Shared include resolver.
    pub resolver: SharedUiMenuResolver,
    /// Random source.
    pub random: Box<dyn UiMenuRandom>,
}

impl UiMenuResolvedParseHost {
    /// Build a resolver-backed host.
    pub fn new(resolver: SharedUiMenuResolver, random: impl UiMenuRandom + 'static) -> UiMenuResolvedParseHost {
        UiMenuResolvedParseHost {
            resolver,
            random: Box::new(random),
        }
    }
}

/// Handle-backed parse host (donor inline `UiMenuParseHost` variant).
pub struct UiMenuHandleParseHost {
    /// Random source.
    pub random: Box<dyn UiMenuRandom>,
    /// Shared handle sources.
    pub sources: SharedUiMenuScriptSources,
    /// Current-operation assertion, invoked around every handle call.
    pub assert_current_operation: UiMenuOperationGuard,
}

impl UiMenuHandleParseHost {
    /// Build a handle-backed host.
    pub fn new(
        random: impl UiMenuRandom + 'static,
        sources: SharedUiMenuScriptSources,
        assert_current_operation: UiMenuOperationGuard,
    ) -> UiMenuHandleParseHost {
        UiMenuHandleParseHost {
            random: Box::new(random),
            sources,
            assert_current_operation,
        }
    }
}

/// Parse host (`UiMenuParseHost`).
pub enum UiMenuParseHost {
    /// Bounded resolver host.
    Resolved(UiMenuResolvedParseHost),
    /// Handle-based host.
    Handle(UiMenuHandleParseHost),
}

/// Streaming menu sink (`UiMenuDefinitionSink`; donor `Promise -> sync`).
pub trait UiMenuDefinitionSink {
    /// Menus published so far.
    fn menu_count(&self) -> usize;
    /// Publish one parsed menu.
    fn publish(&mut self, menu: &UiMenuDefinition) -> Result<(), ClientError>;
}

/// Menu parse options (`UiMenuParseOptions`).
#[derive(Default)]
pub struct UiMenuParseOptions {
    /// Backing memory, defaulting to unaccounted literals.
    pub memory: Option<UiMenuMemoryOwnership>,
    /// Registration sink; without one, registration stays deferred.
    pub registration_sink: Option<Box<dyn UiMenuRegistrationSink>>,
    /// Asset sink.
    pub asset_sink: Option<Box<dyn UiMenuAssetSink>>,
    /// Starting global assets.
    pub initial_assets: Option<UiGlobalAssets>,
    /// Starting font-registered flag.
    pub initial_font_registered: bool,
    /// Streaming menu sink.
    pub menu_sink: Option<Box<dyn UiMenuDefinitionSink>>,
    /// Diagnostic observer.
    pub report_diagnostic: Option<ReportCallback>,
}

/// Menu load plan (`UiMenuLoadPlan`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiMenuLoadPlan {
    /// Load full UI menu sets in order.
    Ui {
        /// Set paths, in load order.
        set_paths: Vec<String>,
    },
    /// Load one HUD menu set through the `COM_Parse` path.
    Hud {
        /// Set path.
        set_path: String,
    },
}

impl UiMenuLoadPlan {
    /// Donor `kind` discriminator.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            UiMenuLoadPlan::Ui { .. } => "ui",
            UiMenuLoadPlan::Hud { .. } => "hud",
        }
    }
}

/// Default full-UI plan: `ui/menus.txt` then `ui/ingame.txt`.
#[must_use]
pub fn default_ui_menu_plan() -> UiMenuLoadPlan {
    UiMenuLoadPlan::Ui {
        set_paths: vec!["ui/menus.txt".to_string(), "ui/ingame.txt".to_string()],
    }
}

/// Default HUD plan: `ui/hud.txt`.
#[must_use]
pub fn default_hud_menu_plan() -> UiMenuLoadPlan {
    UiMenuLoadPlan::Hud {
        set_path: "ui/hud.txt".to_string(),
    }
}
/// A token-record source for one menu file (`UiMenuTokenSource`).
pub trait UiMenuTokenSource {
    /// Canonical source path.
    fn path(&self) -> &str;
    /// Next preprocessed record, or `None` at end of source.
    fn next_record(&mut self) -> Result<Option<ScriptTokenRecord>, ClientError>;
    /// Current file and line.
    fn position(&mut self) -> ScriptSourcePosition;
    /// Release the source.
    fn dispose(&mut self);
}

/// Reader-backed token source.
struct ReaderTokenSource {
    path: String,
    reader: ScriptSourceReader,
}

impl UiMenuTokenSource for ReaderTokenSource {
    /// Canonical source path.
    fn path(&self) -> &str {
        &self.path
    }

    /// Next record, mapping source failures to end of source like the donor.
    fn next_record(&mut self) -> Result<Option<ScriptTokenRecord>, ClientError> {
        match self.reader.next() {
            Ok(record) => Ok(record),
            Err(error) if self.reader.is_source_failure(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Current file and line.
    fn position(&mut self) -> ScriptSourcePosition {
        self.reader.position()
    }

    /// Release the reader.
    fn dispose(&mut self) {
        self.reader.dispose();
    }
}

/// Handle-backed token source.
struct HandleTokenSource {
    path: String,
    handle: i32,
    sources: SharedUiMenuScriptSources,
    guard: UiMenuOperationGuard,
}

impl UiMenuTokenSource for HandleTokenSource {
    /// Requested source path.
    fn path(&self) -> &str {
        &self.path
    }

    /// Next record through the shared handle sources.
    fn next_record(&mut self) -> Result<Option<ScriptTokenRecord>, ClientError> {
        (self.guard)();
        let record = self.sources.read_token_handle(self.handle);
        (self.guard)();
        Ok(record)
    }

    /// Current file and line through the shared handle sources.
    fn position(&mut self) -> ScriptSourcePosition {
        (self.guard)();
        let position = self.sources.source_file_and_line(self.handle);
        (self.guard)();
        position.unwrap_or(ScriptSourcePosition {
            filename: String::new(),
            line: 0,
        })
    }

    /// Free the handle through the shared sources.
    fn dispose(&mut self) {
        (self.guard)();
        self.sources.free_source_handle(self.handle);
        (self.guard)();
    }
}

/// A token cursor over one menu source (`UiMenuTokenCursor`).
pub struct UiMenuTokenCursor {
    source: Box<dyn UiMenuTokenSource>,
}

impl UiMenuTokenCursor {
    /// Open preprocessed text with a shared resolver.
    pub fn open(
        source: ScriptSource,
        resolver: SharedUiMenuResolver,
        options: ScriptPreprocessorOptions,
    ) -> Result<UiMenuTokenCursor, ClientError> {
        let path = source.path.clone();
        let reader = ScriptSourceReader::open(source, resolver, options)?;
        Ok(UiMenuTokenCursor {
            source: Box::new(ReaderTokenSource { path, reader }),
        })
    }

    /// Open a handle source, or `None` when the handle load fails.
    pub fn open_handle(
        path: &str,
        sources: &SharedUiMenuScriptSources,
        assert_current_operation: &UiMenuOperationGuard,
    ) -> Option<UiMenuTokenCursor> {
        assert_current_operation();
        let handle = sources.load_source_handle(path);
        assert_current_operation();
        if handle == 0 {
            return None;
        }
        Some(UiMenuTokenCursor {
            source: Box::new(HandleTokenSource {
                path: path.to_string(),
                handle,
                sources: sources.clone(),
                guard: Rc::clone(assert_current_operation),
            }),
        })
    }

    /// Source path.
    #[must_use]
    pub fn path(&self) -> &str {
        self.source.path()
    }

    /// Current file and line.
    pub fn position(&mut self) -> ScriptSourcePosition {
        self.source.position()
    }

    /// Next token, or `None` at end of source (donor `next`).
    pub fn next_token(&mut self) -> Result<Option<ScriptToken>, ClientError> {
        match self.source.next_record()? {
            Some(record) => Ok(Some(record.token)),
            None => Ok(None),
        }
    }

    /// Release the source.
    pub fn dispose(&mut self) {
        self.source.dispose();
    }
}

/// A root source for parsing: text or a live cursor.
pub enum UiMenuSourceInput {
    /// Preprocessed text source.
    Source(ScriptSource),
    /// Live token cursor (handle hosts).
    Cursor(UiMenuTokenCursor),
}

impl UiMenuSourceInput {
    /// Canonical source path.
    fn path(&self) -> &str {
        match self {
            UiMenuSourceInput::Source(source) => &source.path,
            UiMenuSourceInput::Cursor(cursor) => cursor.path(),
        }
    }
}

/// Recoverable field failure (donor `MenuParseFailure`) versus fatal error.
enum MenuError {
    /// Abort the current field, menu, or asset block; the caller reports.
    Abort,
    /// Abort the whole load (donor `ScriptLanguageError`).
    Fatal(ClientError),
}

impl From<ClientError> for MenuError {
    /// Client errors are always fatal.
    fn from(error: ClientError) -> MenuError {
        MenuError::Fatal(error)
    }
}

/// Shared diagnostics plus chained observer callbacks.
struct ParserReports {
    diagnostics: Vec<ScriptDiagnostic>,
    preprocessor_report: Option<ReportCallback>,
    user_report: Option<ReportCallback>,
}

/// Cloneable preprocessor options template (one fresh reader per file).
struct PreprocessorTemplate {
    initial_defines: Vec<String>,
    global_defines: Option<ScriptGlobalSnapshot>,
    install_builtins: bool,
    now: Option<Rc<RefCell<NowCallback>>>,
    max_include_depth: Option<usize>,
    max_macro_expansions: Option<usize>,
    max_queued_tokens: Option<usize>,
    max_output_tokens: Option<usize>,
    max_source_tokens: Option<usize>,
    max_defines: Option<usize>,
    max_expression_tokens: Option<usize>,
    debug_eval: Option<Rc<RefCell<DebugEvalCallback>>>,
}

impl PreprocessorTemplate {
    /// Capture reusable options; live globals snapshot to detached defines.
    ///
    /// The donor shares one options object (with live globals) across every
    /// reader. Rust readers take options by value, so each open clones this
    /// template; `now`/`debug_eval` closures stay shared through handles.
    fn new(options: ScriptPreprocessorOptions) -> PreprocessorTemplate {
        let ScriptPreprocessorOptions {
            initial_defines,
            globals,
            global_defines,
            install_builtins,
            now,
            max_include_depth,
            max_macro_expansions,
            max_queued_tokens,
            max_output_tokens,
            max_source_tokens,
            max_defines,
            max_expression_tokens,
            report: _,
            debug_eval,
        } = options;
        let had_snapshot = global_defines.is_some();
        let mut merged = global_defines.unwrap_or_default();
        if let Some(globals) = globals.as_ref() {
            let mut definitions = globals.snapshot().definitions;
            definitions.append(&mut merged.definitions);
            merged.definitions = definitions;
        }
        PreprocessorTemplate {
            initial_defines,
            global_defines: if !had_snapshot && merged.definitions.is_empty() {
                None
            } else {
                Some(merged)
            },
            install_builtins,
            now: now.map(|callback| Rc::new(RefCell::new(callback))),
            max_include_depth,
            max_macro_expansions,
            max_queued_tokens,
            max_output_tokens,
            max_source_tokens,
            max_defines,
            max_expression_tokens,
            debug_eval: debug_eval.map(|callback| Rc::new(RefCell::new(callback))),
        }
    }

    /// Build fresh options chaining reports into `reports`.
    fn build(&self, reports: &Rc<RefCell<ParserReports>>) -> ScriptPreprocessorOptions {
        let chained = Rc::clone(reports);
        let now = self.now.as_ref().map(|shared| {
            let shared = Rc::clone(shared);
            Box::new(move || shared.borrow_mut()()) as NowCallback
        });
        let debug_eval = self.debug_eval.as_ref().map(|shared| {
            let shared = Rc::clone(shared);
            Box::new(move |line: &str| shared.borrow_mut()(line)) as DebugEvalCallback
        });
        ScriptPreprocessorOptions {
            initial_defines: self.initial_defines.clone(),
            globals: None,
            global_defines: self.global_defines.clone(),
            install_builtins: self.install_builtins,
            now,
            max_include_depth: self.max_include_depth,
            max_macro_expansions: self.max_macro_expansions,
            max_queued_tokens: self.max_queued_tokens,
            max_output_tokens: self.max_output_tokens,
            max_source_tokens: self.max_source_tokens,
            max_defines: self.max_defines,
            max_expression_tokens: self.max_expression_tokens,
            report: Some(Box::new(move |diagnostic: &ScriptDiagnostic| {
                let mut shared = chained.borrow_mut();
                if let Some(report) = shared.preprocessor_report.as_mut() {
                    report(diagnostic);
                }
                shared.diagnostics.push(diagnostic.clone());
                if let Some(report) = shared.user_report.as_mut() {
                    report(diagnostic);
                }
            })),
            debug_eval,
        }
    }
}

/// Menu definition source parser (`UiMenuSourceParser`).
pub struct UiMenuSourceParser {
    host: UiMenuParseHost,
    template: PreprocessorTemplate,
    reports: Rc<RefCell<ParserReports>>,
    memory: UiMenuMemoryOwnership,
    storage: SharedUiMenuMemory,
    menus: Vec<UiMenuDefinition>,
    loaded_files: Vec<String>,
    assets: UiGlobalAssets,
    registrations: Vec<UiMenuRegistrationEvent>,
    font_registered: bool,
    registration_sink: Option<Box<dyn UiMenuRegistrationSink>>,
    asset_sink: Option<Box<dyn UiMenuAssetSink>>,
    menu_sink: Option<Box<dyn UiMenuDefinitionSink>>,
}

impl UiMenuSourceParser {
    /// Build a parser over `host` (donor constructor).
    pub fn new(
        host: UiMenuParseHost,
        mut preprocessor_options: ScriptPreprocessorOptions,
        parse_options: UiMenuParseOptions,
    ) -> UiMenuSourceParser {
        let UiMenuParseOptions {
            memory,
            registration_sink,
            asset_sink,
            initial_assets,
            initial_font_registered,
            menu_sink,
            report_diagnostic,
        } = parse_options;
        let preprocessor_report = preprocessor_options.report.take();
        let memory = memory.unwrap_or(UiMenuMemoryOwnership::Unaccounted);
        let storage = match &memory {
            UiMenuMemoryOwnership::Qvm32 { memory } => memory.clone(),
            UiMenuMemoryOwnership::Unaccounted => SharedUiMenuMemory::new(MenuArenaMemory::new()),
        };
        UiMenuSourceParser {
            host,
            template: PreprocessorTemplate::new(preprocessor_options),
            reports: Rc::new(RefCell::new(ParserReports {
                diagnostics: Vec::new(),
                preprocessor_report,
                user_report: report_diagnostic,
            })),
            memory,
            storage,
            menus: Vec::new(),
            loaded_files: Vec::new(),
            assets: initial_assets.unwrap_or_default(),
            registrations: Vec::new(),
            font_registered: initial_font_registered,
            registration_sink,
            asset_sink,
            menu_sink,
        }
    }

    /// Load one plan into definitions (donor `load`; `Promise -> sync`).
    pub fn load(&mut self, plan: &UiMenuLoadPlan) -> Result<UiMenuDefinitions, ClientError> {
        match plan {
            UiMenuLoadPlan::Ui { set_paths } => {
                for path in set_paths {
                    self.load_set(path, false)?;
                }
            }
            UiMenuLoadPlan::Hud { set_path } => {
                self.load_set(set_path, true)?;
            }
        }
        Ok(UiMenuDefinitions {
            memory: self.memory.clone(),
            menus: self.menus.clone(),
            assets: self.assets.clone(),
            loaded_files: self.loaded_files.clone(),
            diagnostics: self.reports.borrow().diagnostics.clone(),
            registration: if self.registration_sink.is_none() {
                UiMenuRegistrationState::Deferred {
                    events: self.registrations.clone(),
                }
            } else {
                UiMenuRegistrationState::Completed {
                    events: self.registrations.clone(),
                }
            },
            font_registered: self.font_registered,
        })
    }

    /// Parse one root source (donor `parseSource`; `Promise -> sync`).
    pub fn parse_source(&mut self, source: UiMenuSourceInput, hud: bool) -> Result<(), ClientError> {
        if source.path().is_empty() {
            return Err(self.fail(
                "menu file resolver returned an empty canonical path",
                &SourceLocation {
                    path: String::new(),
                    line: 1,
                    column: 1,
                },
            ));
        }
        if self.menu_sink.is_none() {
            self.loaded_files.push(source.path().to_string());
        } else {
            self.reports.borrow_mut().diagnostics.clear();
        }
        let mut cursor = self.open_source(source)?;
        while let Some(token) = cursor.next_token()? {
            if token_value(&token).starts_with('}') {
                break;
            }
            let keyword = token_value(&token).to_lowercase();
            if keyword == "assetglobaldef" {
                match self.parse_assets(&mut cursor, token.location(), hud) {
                    Ok(()) => {}
                    Err(MenuError::Abort) => break,
                    Err(MenuError::Fatal(error)) => return Err(error),
                }
            } else if keyword == "menudef" {
                let index = match &self.menu_sink {
                    None => self.menus.len(),
                    Some(sink) => sink.menu_count(),
                };
                if index >= MAX_UI_MENUS {
                    if self.menu_sink.is_none() {
                        return Err(self.fail(&format!("more than {MAX_UI_MENUS} menus defined"), token.location()));
                    }
                    continue;
                }
                let menu = match self.parse_menu(&mut cursor, token.location(), index)? {
                    Some(menu) => menu,
                    None => continue,
                };
                // The donor guards a reentrant registration advancing the
                // count mid-parse; sync sinks cannot reenter, so the parsed
                // menu is always the published one.
                match &mut self.menu_sink {
                    None => self.menus.push(menu),
                    Some(sink) => sink.publish(&menu)?,
                }
            }
        }
        cursor.dispose();
        Ok(())
    }

    /// Record one registration, retaining it unless streaming (donor `register`).
    fn register(&mut self, event: UiMenuRegistrationEvent) -> Result<UiMenuRegistrationResult, ClientError> {
        if self.menu_sink.is_none() {
            self.registrations.push(event.clone());
        }
        match &mut self.registration_sink {
            Some(sink) => sink.register(&event),
            None => Ok(None),
        }
    }

    /// Load one menu set through the token path or the HUD path.
    fn load_set(&mut self, path: &str, hud: bool) -> Result<(), ClientError> {
        if hud && !matches!(self.host, UiMenuParseHost::Resolved(_)) {
            return Err(ClientError::BadUi(
                "HUD menu sets require a bounded COM text resolver".to_string(),
            ));
        }
        let source = self.resolve_root(path, "menu set")?;
        self.loaded_files.push(source.path().to_string());
        if hud {
            if let UiMenuSourceInput::Source(text) = &source {
                if text.text.encode_utf16().count() > MAX_HUD_MENU_SET_BYTES {
                    return Err(self.fail(
                        &format!("menu file too large: {}", text.path),
                        &SourceLocation {
                            path: text.path.clone(),
                            line: 1,
                            column: 1,
                        },
                    ));
                }
                let UiMenuSourceInput::Source(text) = source else {
                    return Err(ClientError::BadUi("HUD menu set changed kind during load".to_string()));
                };
                return self.load_hud_set(&text);
            }
        }
        let mut cursor = self.open_source(source)?;
        'menu_set: while let Some(token) = cursor.next_token()? {
            let value = token_value(&token);
            if value.is_empty() || value.starts_with('}') {
                break;
            }
            if !value.eq_ignore_ascii_case("loadmenu") {
                continue;
            }
            let opening = match cursor.next_token()? {
                Some(token) => token,
                None => break,
            };
            if !token_value(&opening).starts_with('{') {
                break;
            }
            loop {
                let file = match cursor.next_token()? {
                    Some(token) => token,
                    None => break 'menu_set,
                };
                let value = token_value(&file);
                if value.is_empty() {
                    break 'menu_set;
                }
                if value.starts_with('}') {
                    break;
                }
                self.load_menu_file(&value, false)?;
            }
        }
        cursor.dispose();
        Ok(())
    }

    /// Load one HUD menu set through `COM_Parse` tokens (donor `loadHudSet`).
    fn load_hud_set(&mut self, source: &ScriptSource) -> Result<(), ClientError> {
        let compressed = compress_common_text(&source.text)?;
        let mut offset = Some(0);
        loop {
            let token = latin1_to_string(&com_parse_token(&compressed, &mut offset)?);
            if token.is_empty() || token.starts_with('}') {
                return Ok(());
            }
            if !token.eq_ignore_ascii_case("loadmenu") {
                continue;
            }
            let opening = latin1_to_string(&com_parse_token(&compressed, &mut offset)?);
            if !opening.starts_with('{') {
                return Ok(());
            }
            loop {
                let path = latin1_to_string(&com_parse_token(&compressed, &mut offset)?);
                if path == "}" {
                    break;
                }
                if path.is_empty() {
                    return Ok(());
                }
                self.load_menu_file(&path, true)?;
            }
        }
    }

    /// Open a root input as a cursor, chaining preprocessor reports.
    fn open_source(&mut self, source: UiMenuSourceInput) -> Result<UiMenuTokenCursor, ClientError> {
        match source {
            UiMenuSourceInput::Cursor(cursor) => Ok(cursor),
            UiMenuSourceInput::Source(source) => match &self.host {
                UiMenuParseHost::Resolved(host) => {
                    let options = self.template.build(&self.reports);
                    UiMenuTokenCursor::open(source, host.resolver.clone(), options)
                }
                UiMenuParseHost::Handle(_) => Err(ClientError::BadUi(
                    "Parsed text sources require a diagnostic menu resolver".to_string(),
                )),
            },
        }
    }

    /// Load one menu file, with the HUD `testhud` fallback (donor `loadMenuFile`).
    fn load_menu_file(&mut self, path: &str, hud: bool) -> Result<(), ClientError> {
        let mut source = self.open_file(path);
        if source.is_none() && hud && path != "ui/testhud.menu" {
            source = self.open_file("ui/testhud.menu");
        }
        match source {
            Some(source) => self.parse_source(source, hud),
            None => Ok(()),
        }
    }

    /// Parse one `assetGlobalDef` block (donor `parseAssets`).
    fn parse_assets(
        &mut self,
        cursor: &mut UiMenuTokenCursor,
        start: &SourceLocation,
        hud: bool,
    ) -> Result<(), MenuError> {
        self.expect(cursor, "{", start)?;
        loop {
            let field = self.expect_any(cursor, start, false)?;
            let value = token_value(&field);
            if value == "}" {
                return Ok(());
            }
            match value.to_lowercase().as_str() {
                "font" => {
                    let reference = self.read_font(cursor, field.location())?;
                    self.register(UiMenuRegistrationEvent::Font {
                        reference: reference.clone(),
                        location: field.location().clone(),
                    })?;
                    self.assets.text_font = Some(reference.clone());
                    self.publish_asset(UiMenuAssetPublication::TextFont(reference));
                    if !hud {
                        self.font_registered = true;
                        self.publish_asset(UiMenuAssetPublication::FontRegistered(true));
                    }
                }
                "smallfont" => {
                    let reference = self.read_font(cursor, field.location())?;
                    self.register(UiMenuRegistrationEvent::Font {
                        reference: reference.clone(),
                        location: field.location().clone(),
                    })?;
                    self.assets.small_font = Some(reference.clone());
                    self.publish_asset(UiMenuAssetPublication::SmallFont(reference));
                }
                "bigfont" => {
                    let reference = self.read_font(cursor, field.location())?;
                    self.register(UiMenuRegistrationEvent::Font {
                        reference: reference.clone(),
                        location: field.location().clone(),
                    })?;
                    self.assets.big_font = Some(reference.clone());
                    self.publish_asset(UiMenuAssetPublication::BigFont(reference));
                }
                "gradientbar" => {
                    let reference = UiShaderReference {
                        path: self.read_string(cursor, field.location())?,
                    };
                    self.register(UiMenuRegistrationEvent::Picture {
                        reference: reference.clone(),
                        location: field.location().clone(),
                    })?;
                    self.assets.gradient_bar = Some(reference.clone());
                    self.publish_asset(UiMenuAssetPublication::GradientBar(reference));
                }
                "menuentersound" => {
                    let reference = UiSoundReference {
                        path: self.read_string(cursor, field.location())?,
                    };
                    self.register(UiMenuRegistrationEvent::Sound {
                        reference: reference.clone(),
                        location: field.location().clone(),
                    })?;
                    self.assets.menu_enter_sound = Some(reference.clone());
                    self.publish_asset(UiMenuAssetPublication::MenuEnterSound(reference));
                }
                "menuexitsound" => {
                    let reference = UiSoundReference {
                        path: self.read_string(cursor, field.location())?,
                    };
                    self.register(UiMenuRegistrationEvent::Sound {
                        reference: reference.clone(),
                        location: field.location().clone(),
                    })?;
                    self.assets.menu_exit_sound = Some(reference.clone());
                    self.publish_asset(UiMenuAssetPublication::MenuExitSound(reference));
                }
                "itemfocussound" => {
                    let reference = UiSoundReference {
                        path: self.read_string(cursor, field.location())?,
                    };
                    self.register(UiMenuRegistrationEvent::Sound {
                        reference: reference.clone(),
                        location: field.location().clone(),
                    })?;
                    self.assets.item_focus_sound = Some(reference.clone());
                    self.publish_asset(UiMenuAssetPublication::ItemFocusSound(reference));
                }
                "menubuzzsound" => {
                    let reference = UiSoundReference {
                        path: self.read_string(cursor, field.location())?,
                    };
                    self.register(UiMenuRegistrationEvent::Sound {
                        reference: reference.clone(),
                        location: field.location().clone(),
                    })?;
                    self.assets.menu_buzz_sound = Some(reference.clone());
                    self.publish_asset(UiMenuAssetPublication::MenuBuzzSound(reference));
                }
                "cursor" => {
                    let path = self.read_string_reference(cursor, field.location())?;
                    self.publish_asset(UiMenuAssetPublication::CursorStr(path.clone()));
                    let reference = UiShaderReference {
                        path: path.as_ref().map(UiStringReference::read),
                    };
                    self.register(UiMenuRegistrationEvent::Picture {
                        reference: reference.clone(),
                        location: field.location().clone(),
                    })?;
                    self.assets.cursor = Some(reference.clone());
                    self.publish_asset(UiMenuAssetPublication::Cursor(reference));
                }
                "fadeclamp" => {
                    let value = self.read_float(cursor, field.location())?;
                    self.assets.fade_clamp = value;
                    self.publish_asset(UiMenuAssetPublication::FadeClamp(value));
                }
                "fadecycle" => {
                    let value = self.read_int(cursor, field.location())?;
                    self.assets.fade_cycle = value;
                    self.publish_asset(UiMenuAssetPublication::FadeCycle(value));
                }
                "fadeamount" => {
                    let value = self.read_float(cursor, field.location())?;
                    self.assets.fade_amount = value;
                    self.publish_asset(UiMenuAssetPublication::FadeAmount(value));
                }
                "shadowx" => {
                    let value = self.read_float(cursor, field.location())?;
                    self.assets.shadow_x = value;
                    self.publish_asset(UiMenuAssetPublication::ShadowX(value));
                }
                "shadowy" => {
                    let value = self.read_float(cursor, field.location())?;
                    self.assets.shadow_y = value;
                    self.publish_asset(UiMenuAssetPublication::ShadowY(value));
                }
                "shadowcolor" => {
                    for component in [
                        UiColorComponent::X,
                        UiColorComponent::Y,
                        UiColorComponent::Z,
                        UiColorComponent::W,
                    ] {
                        let value = self.read_float(cursor, field.location())?;
                        match component {
                            UiColorComponent::X => self.assets.shadow_color.x = value,
                            UiColorComponent::Y => self.assets.shadow_color.y = value,
                            UiColorComponent::Z => self.assets.shadow_color.z = value,
                            UiColorComponent::W => self.assets.shadow_color.w = value,
                        }
                        self.publish_asset(UiMenuAssetPublication::ShadowColorComponent { component, value });
                    }
                    self.assets.shadow_fade_clamp = self.assets.shadow_color.w;
                    self.publish_asset(UiMenuAssetPublication::ShadowFadeClamp(self.assets.shadow_fade_clamp));
                }
                _ => self.warn(&format!("unknown asset keyword {value}"), field.location()),
            }
        }
    }
    /// Parse one `menuDef` block (donor `parseMenu`).
    fn parse_menu(
        &mut self,
        cursor: &mut UiMenuTokenCursor,
        start: &SourceLocation,
        index: usize,
    ) -> Result<Option<UiMenuDefinition>, ClientError> {
        let menu = UiMenuDefinition::new(self.storage.menu_record(index), start.clone());
        menu.initialize(&self.assets);
        let opening = match cursor.next_token()? {
            Some(token) => token,
            None => return Ok(None),
        };
        if !token_value(&opening).starts_with('{') {
            return Ok(None);
        }
        loop {
            let field = match cursor.next_token()? {
                Some(token) => token,
                None => {
                    self.source_error("end of file inside menu", start, cursor);
                    return Ok(None);
                }
            };
            let value = token_value(&field);
            if value.starts_with('}') {
                self.post_parse(&menu);
                return Ok(Some(menu));
            }
            if let Err(error) = self.parse_menu_field(&menu, cursor, &value, field.location()) {
                match error {
                    MenuError::Abort => {
                        self.source_error(
                            &format!("couldn't parse menu keyword {value}"),
                            field.location(),
                            cursor,
                        );
                        return Ok(None);
                    }
                    MenuError::Fatal(error) => return Err(error),
                }
            }
        }
    }

    /// Parse one menu keyword (donor `parseMenu` switch).
    fn parse_menu_field(
        &mut self,
        menu: &UiMenuDefinition,
        cursor: &mut UiMenuTokenCursor,
        value: &str,
        at: &SourceLocation,
    ) -> Result<(), MenuError> {
        let window = menu.window();
        match value.to_lowercase().as_str() {
            "font" => {
                menu.set_font(self.read_string_reference(cursor, at)?);
                if !self.font_registered {
                    let reference = UiFontReference {
                        path: menu.font(),
                        point_size: 48,
                    };
                    self.register(UiMenuRegistrationEvent::Font {
                        reference: reference.clone(),
                        location: at.clone(),
                    })?;
                    self.assets.text_font = Some(reference.clone());
                    self.publish_asset(UiMenuAssetPublication::TextFont(reference));
                    self.font_registered = true;
                    self.publish_asset(UiMenuAssetPublication::FontRegistered(true));
                }
            }
            "name" => {
                window.set_name(self.read_string_reference(cursor, at)?);
            }
            "fullscreen" => menu.set_full_screen(self.read_int(cursor, at)?),
            "rect" => self.read_rect_into(&menu.window().rect(), cursor, at)?,
            "style" => window.set_style(self.read_int(cursor, at)?),
            "visible" => {
                if self.read_int(cursor, at)? != 0 {
                    window.set_flags(window.flags() | UiWindowFlag::VISIBLE);
                }
            }
            "onopen" => menu.set_on_open(self.read_script(cursor, at)?),
            "onclose" => menu.set_on_close(self.read_script(cursor, at)?),
            "onesc" => menu.set_on_escape(self.read_script(cursor, at)?),
            "border" => window.set_border(self.read_int(cursor, at)?),
            "bordersize" => window.set_border_size(self.read_float(cursor, at)?),
            "backcolor" => self.read_color_into(&window.back_color(), cursor, at)?,
            "forecolor" => self.read_fore_color(&window, cursor, at)?,
            "bordercolor" => self.read_color_into(&window.border_color(), cursor, at)?,
            "focuscolor" => self.read_color_into(&menu.focus_color(), cursor, at)?,
            "disablecolor" => self.read_color_into(&menu.disable_color(), cursor, at)?,
            "outlinecolor" => self.read_color_into(&window.outline_color(), cursor, at)?,
            "background" => {
                let reference = UiShaderReference {
                    path: self.read_string(cursor, at)?,
                };
                let registration = self.register(UiMenuRegistrationEvent::Picture {
                    reference: reference.clone(),
                    location: at.clone(),
                })?;
                window.set_background(reference, registration.map(|registration| registration.handle));
            }
            "ownerdraw" => window.set_owner_draw(self.read_int(cursor, at)?),
            "ownerdrawflag" => window.set_owner_draw_flags(window.owner_draw_flags() | self.read_int(cursor, at)?),
            "outofboundsclick" => {
                window.set_flags(window.flags() | UiWindowFlag::OUT_OF_BOUNDS_CLICK);
            }
            "soundloop" => {
                menu.set_sound_loop(self.read_string_reference(cursor, at)?);
            }
            "itemdef" => {
                if menu.item_count() < MAX_UI_MENU_ITEMS as i32 {
                    let count = menu.item_count().max(0) as usize;
                    match self.parse_item(cursor, at, menu)? {
                        Some(_) => {}
                        None => return Err(MenuError::Abort),
                    }
                    if let Some(item) = menu.slot_item(count) {
                        if item.item_type() == UiItemTypeCode::ListBox as i32 {
                            item.set_cursor_position(0);
                            if let Some(list) = item.list_data() {
                                // The donor writes the cursor twice; preserved.
                                list.set_cursor_position(0);
                                list.set_start_position(0);
                                list.set_end_position(0);
                                list.set_cursor_position(0);
                            }
                        }
                    }
                    let parent_item = menu.slot_item(count);
                    menu.set_item_count(menu.item_count() + 1);
                    match parent_item {
                        Some(parent_item) => {
                            parent_item.set_parent(Some(menu.clone()));
                        }
                        None => {
                            return Err(MenuError::Fatal(
                                self.fail("MenuParse_itemDef dereferences a NULL retained item parent", at),
                            ));
                        }
                    }
                }
            }
            "cinematic" => {
                window.set_cinematic(self.read_string_reference(cursor, at)?);
            }
            "popup" => window.set_flags(window.flags() | UiWindowFlag::POPUP),
            "fadeclamp" => menu.set_fade_clamp(self.read_float(cursor, at)?),
            "fadecycle" => menu.set_fade_cycle(self.read_int(cursor, at)?),
            "fadeamount" => menu.set_fade_amount(self.read_float(cursor, at)?),
            _ => self.source_error(&format!("unknown menu keyword {value}"), at, cursor),
        }
        Ok(())
    }

    /// Parse one `itemDef` block (donor `parseItem`).
    fn parse_item(
        &mut self,
        cursor: &mut UiMenuTokenCursor,
        start: &SourceLocation,
        menu: &UiMenuDefinition,
    ) -> Result<Option<UiItemDefinition>, ClientError> {
        // The QVM32 `itemDef_t` is 540 bytes; `Menus[]` itself is static.
        let allocation = match self.allocate(ITEM_SIZE) {
            Some(allocation) => allocation,
            None => {
                let count = menu.item_count().max(0) as usize;
                if count < MAX_UI_MENU_ITEMS {
                    menu.set_item(count, None);
                }
                return Err(self.fail("Item_Init dereferences a failed UI_Alloc itemDef_t", start));
            }
        };
        let allocation_offset = match self.memory {
            UiMenuMemoryOwnership::Qvm32 { .. } => Some(allocation.offset()),
            UiMenuMemoryOwnership::Unaccounted => None,
        };
        let item = UiItemDefinition::new(allocation, start.clone(), allocation_offset);
        menu.set_item(menu.item_count().max(0) as usize, Some(item.clone()));
        item.initialize();
        let opening = match cursor.next_token()? {
            Some(token) => token,
            None => return Ok(None),
        };
        if !token_value(&opening).starts_with('{') {
            return Ok(None);
        }
        loop {
            let field = match cursor.next_token()? {
                Some(token) => token,
                None => {
                    self.source_error("end of file inside menu item", start, cursor);
                    return Ok(None);
                }
            };
            let value = token_value(&field);
            if value.starts_with('}') {
                return Ok(Some(item));
            }
            if let Err(error) = self.parse_item_field(&item, cursor, &value.to_lowercase(), &value, field.location()) {
                match error {
                    MenuError::Abort => {
                        self.source_error(
                            &format!("couldn't parse menu item keyword {value}"),
                            field.location(),
                            cursor,
                        );
                        return Ok(None);
                    }
                    MenuError::Fatal(error) => return Err(error),
                }
            }
        }
    }
    /// Parse one item keyword (donor `parseItemField`).
    #[allow(clippy::too_many_lines)]
    fn parse_item_field(
        &mut self,
        item: &UiItemDefinition,
        cursor: &mut UiMenuTokenCursor,
        keyword: &str,
        original_keyword: &str,
        at: &SourceLocation,
    ) -> Result<(), MenuError> {
        let window = item.window();
        match keyword {
            "name" => item.window().set_name(self.read_string_reference(cursor, at)?),
            "text" => item.set_text(self.read_string_reference(cursor, at)?),
            "group" => window.set_group(self.read_string_reference(cursor, at)?),
            "asset_model" => {
                self.validate_type_data(item)?;
                let data = item.model_data();
                let reference = UiModelReference {
                    path: self.read_string(cursor, at)?,
                };
                let registration = self.register(UiMenuRegistrationEvent::Model {
                    reference: reference.clone(),
                    location: at.clone(),
                })?;
                item.set_asset(
                    UiMenuResource::Model(reference),
                    registration.map(|registration| registration.handle),
                );
                let random = self.next_random();
                if !(0..=0x7fff).contains(&random) {
                    return Err(MenuError::Fatal(
                        self.fail("UI random must return an integer in [0, 32767]", at),
                    ));
                }
                match data {
                    Some(data) => data.set_angle(random % 360),
                    None => {
                        return Err(MenuError::Fatal(
                            self.fail("ItemParse_asset_model dereferences NULL model typeData", at),
                        ))
                    }
                }
            }
            "asset_shader" => {
                let reference = UiShaderReference {
                    path: self.read_string(cursor, at)?,
                };
                let registration = self.register(UiMenuRegistrationEvent::Picture {
                    reference: reference.clone(),
                    location: at.clone(),
                })?;
                item.set_asset(
                    UiMenuResource::Shader(reference),
                    registration.map(|registration| registration.handle),
                );
            }
            "model_origin" => {
                let data = self.require_model(item, at)?;
                let origin = data.origin();
                origin.set_x(self.read_float(cursor, at)?);
                origin.set_y(self.read_float(cursor, at)?);
                origin.set_z(self.read_float(cursor, at)?);
            }
            "model_fovx" => self
                .require_model(item, at)?
                .set_field_of_view_x(self.read_float(cursor, at)?),
            "model_fovy" => self
                .require_model(item, at)?
                .set_field_of_view_y(self.read_float(cursor, at)?),
            "model_rotation" => self
                .require_model(item, at)?
                .set_rotation_speed(self.read_int(cursor, at)?),
            "model_angle" => self.require_model(item, at)?.set_angle(self.read_int(cursor, at)?),
            "rect" => self.read_rect_into(&window.client_rect(), cursor, at)?,
            "style" => window.set_style(self.read_int(cursor, at)?),
            "decoration" => window.set_flags(window.flags() | UiWindowFlag::DECORATION),
            "notselectable" => {
                self.validate_type_data(item)?;
                if item.item_type() == UiItemTypeCode::ListBox as i32 {
                    if let Some(list) = item.list_data() {
                        list.set_not_selectable(true);
                    }
                }
            }
            "wrapped" => window.set_flags(window.flags() | UiWindowFlag::WRAPPED),
            "autowrapped" => window.set_flags(window.flags() | UiWindowFlag::AUTO_WRAPPED),
            "horizontalscroll" => window.set_flags(window.flags() | UiWindowFlag::HORIZONTAL),
            "type" => {
                item.set_item_type(self.read_int(cursor, at)?);
                self.validate_type_data(item)?;
            }
            "elementwidth" => self
                .require_list(item, at)?
                .set_element_width(self.read_float(cursor, at)?),
            "elementheight" => self
                .require_list(item, at)?
                .set_element_height(self.read_float(cursor, at)?),
            "feeder" => item.set_special(self.read_float(cursor, at)?),
            "elementtype" => self.checked_list(item)?.set_element_style(self.read_int(cursor, at)?),
            "columns" => {
                let list = self.checked_list(item)?;
                self.read_columns(&list, cursor, at)?;
            }
            "border" => window.set_border(self.read_int(cursor, at)?),
            "bordersize" => window.set_border_size(self.read_float(cursor, at)?),
            "visible" => {
                if self.read_int(cursor, at)? != 0 {
                    window.set_flags(window.flags() | UiWindowFlag::VISIBLE);
                }
            }
            "ownerdraw" => {
                window.set_owner_draw(self.read_int(cursor, at)?);
                // ItemParse_ownerdraw changes type without clearing or
                // validating typeData.
                item.set_item_type(UiItemTypeCode::OwnerDraw as i32);
            }
            "align" => item.set_alignment(self.read_int(cursor, at)?),
            "textalign" => item.set_text_alignment(self.read_int(cursor, at)?),
            "textalignx" => item.set_text_align_x(self.read_float(cursor, at)?),
            "textaligny" => item.set_text_align_y(self.read_float(cursor, at)?),
            "textscale" => item.set_text_scale(self.read_float(cursor, at)?),
            "textstyle" => item.set_text_style(self.read_int(cursor, at)?),
            "backcolor" => self.read_color_into(&window.back_color(), cursor, at)?,
            "forecolor" => self.read_fore_color(&window, cursor, at)?,
            "bordercolor" => self.read_color_into(&window.border_color(), cursor, at)?,
            "outlinecolor" => self.read_color_into(&window.outline_color(), cursor, at)?,
            "background" => {
                let reference = UiShaderReference {
                    path: self.read_string(cursor, at)?,
                };
                let registration = self.register(UiMenuRegistrationEvent::Picture {
                    reference: reference.clone(),
                    location: at.clone(),
                })?;
                window.set_background(reference, registration.map(|registration| registration.handle));
            }
            "onfocus" => item.set_on_focus(self.read_script(cursor, at)?),
            "leavefocus" => item.set_leave_focus(self.read_script(cursor, at)?),
            "mouseenter" => item.set_mouse_enter(self.read_script(cursor, at)?),
            "mouseexit" => item.set_mouse_exit(self.read_script(cursor, at)?),
            "mouseentertext" => item.set_mouse_enter_text(self.read_script(cursor, at)?),
            "mouseexittext" => item.set_mouse_exit_text(self.read_script(cursor, at)?),
            "action" => item.set_action(self.read_script(cursor, at)?),
            "special" => item.set_special(self.read_float(cursor, at)?),
            "cvar" => {
                self.validate_type_data(item)?;
                item.set_cvar(self.read_string_reference(cursor, at)?);
                if let Some(edit) = item.edit_data() {
                    edit.set_minimum(-1.0);
                    edit.set_maximum(-1.0);
                    edit.set_default_value(-1.0);
                }
            }
            "maxchars" => self.require_edit(item)?.set_max_chars(self.read_int(cursor, at)?),
            "maxpaintchars" => self.require_edit(item)?.set_max_paint_chars(self.read_int(cursor, at)?),
            "focussound" => {
                let reference = UiSoundReference {
                    path: self.read_string(cursor, at)?,
                };
                let registration = self.register(UiMenuRegistrationEvent::Sound {
                    reference: reference.clone(),
                    location: at.clone(),
                })?;
                item.set_focus_sound(reference, registration.map(|registration| registration.handle));
            }
            "cvarfloat" => {
                let edit = self.require_edit(item)?;
                item.set_cvar(self.read_string_reference(cursor, at)?);
                edit.set_default_value(self.read_float(cursor, at)?);
                edit.set_minimum(self.read_float(cursor, at)?);
                edit.set_maximum(self.read_float(cursor, at)?);
            }
            "cvarstrlist" => {
                let multi = self.require_multi(item)?;
                self.read_string_choices(&multi, cursor, at)?;
            }
            "cvarfloatlist" => {
                let multi = self.require_multi(item)?;
                self.read_number_choices(&multi, cursor, at)?;
            }
            "addcolorrange" => {
                let range = UiColorRange {
                    low: self.read_float(cursor, at)?,
                    high: self.read_float(cursor, at)?,
                    color: self.read_color(cursor, at)?,
                };
                item.add_color_range(&range);
            }
            "ownerdrawflag" => window.set_owner_draw_flags(window.owner_draw_flags() | self.read_int(cursor, at)?),
            "enablecvar" => item.set_cvar_rule(Some(UiCvarRule::Enable {
                script: self.read_script(cursor, at)?,
            })),
            "cvartest" => item.set_cvar_test(self.read_string_reference(cursor, at)?),
            "disablecvar" => item.set_cvar_rule(Some(UiCvarRule::Disable {
                script: self.read_script(cursor, at)?,
            })),
            "showcvar" => item.set_cvar_rule(Some(UiCvarRule::Show {
                script: self.read_script(cursor, at)?,
            })),
            "hidecvar" => item.set_cvar_rule(Some(UiCvarRule::Hide {
                script: self.read_script(cursor, at)?,
            })),
            "cinematic" => window.set_cinematic(self.read_string_reference(cursor, at)?),
            "doubleclick" => self.checked_list(item)?.set_double_click(self.read_script(cursor, at)?),
            _ => self.source_error(&format!("unknown menu item keyword {original_keyword}"), at, cursor),
        }
        Ok(())
    }

    /// Allocate type data for typed items (donor `validateTypeData`).
    fn validate_type_data(&self, item: &UiItemDefinition) -> Result<(), ClientError> {
        if item.has_type_data() {
            return Ok(());
        }
        let item_type = item.item_type();
        let kind = if item_type == UiItemTypeCode::ListBox as i32 {
            Some("list")
        } else if item_type == UiItemTypeCode::Model as i32 {
            Some("model")
        } else if item_type == UiItemTypeCode::Multi as i32 {
            Some("multi")
        } else if is_edit_type(item_type) {
            Some("edit")
        } else {
            None
        };
        let Some(kind) = kind else {
            return Ok(());
        };
        // Item_ValidateTypeData publishes the pointer before the edit/list
        // memset.
        let allocated = self.allocate(if kind == "list" {
            LIST_SIZE
        } else if kind == "multi" {
            MULTI_SIZE
        } else {
            EDIT_SIZE
        });
        item.set_type_data(allocated.clone());
        if kind == "edit" || kind == "list" {
            match allocated {
                Some(allocated) => {
                    allocated.clear();
                    if item_type == UiItemTypeCode::EditField as i32 {
                        allocated.set_i32(EDIT_MAX_PAINT_CHARS, 256);
                    }
                }
                None => {
                    return Err(self.fail(
                        "Item_ValidateTypeData memset dereferences a failed UI_Alloc",
                        item.location(),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Allocate a storage span, or `None` on exhaustion.
    fn allocate(&self, size: usize) -> Option<UiMemoryAllocation> {
        self.storage
            .allocate(size)
            .map(|offset| self.storage.borrow(offset, size))
    }

    /// Next QVM random integer.
    fn next_random(&mut self) -> i32 {
        match &mut self.host {
            UiMenuParseHost::Resolved(host) => host.random.next_int(),
            UiMenuParseHost::Handle(host) => host.random.next_int(),
        }
    }

    /// Required edit type data, aborting the field on null (donor `requireEdit`).
    fn require_edit(&self, item: &UiItemDefinition) -> Result<UiEditFieldDefinition, MenuError> {
        self.validate_type_data(item)?;
        match item.edit_data() {
            Some(edit) => Ok(edit),
            None => Err(MenuError::Abort),
        }
    }

    /// Required list type data, failing the load on null (donor `requireList`).
    fn require_list(&self, item: &UiItemDefinition, at: &SourceLocation) -> Result<UiListBoxDefinition, ClientError> {
        self.validate_type_data(item)?;
        match item.list_data() {
            Some(list) => Ok(list),
            None => Err(self.fail("list parser dereferences NULL typeData", at)),
        }
    }

    /// Required list type data, aborting the field on null (donor `checkedList`).
    fn checked_list(&self, item: &UiItemDefinition) -> Result<UiListBoxDefinition, MenuError> {
        self.validate_type_data(item)?;
        match item.list_data() {
            Some(list) => Ok(list),
            None => Err(MenuError::Abort),
        }
    }

    /// Required multi type data, aborting the field on null (donor `requireMulti`).
    fn require_multi(&self, item: &UiItemDefinition) -> Result<UiMultiDefinition, MenuError> {
        self.validate_type_data(item)?;
        match item.multi_data() {
            Some(multi) => Ok(multi),
            None => Err(MenuError::Abort),
        }
    }

    /// Required model type data, failing the load on null (donor `requireModel`).
    fn require_model(&self, item: &UiItemDefinition, at: &SourceLocation) -> Result<UiModelDefinition, ClientError> {
        self.validate_type_data(item)?;
        match item.model_data() {
            Some(model) => Ok(model),
            None => Err(self.fail("model parser dereferences NULL typeData", at)),
        }
    }

    /// Parse a `columns` count plus rows, clamped to 16 (donor `readColumns`).
    fn read_columns(
        &self,
        list: &UiListBoxDefinition,
        cursor: &mut UiMenuTokenCursor,
        at: &SourceLocation,
    ) -> Result<(), MenuError> {
        let mut count = self.read_int(cursor, at)?;
        if count > MAX_UI_LIST_COLUMNS as i32 {
            count = MAX_UI_LIST_COLUMNS as i32;
        }
        list.set_column_count(count);
        for index in 0..count.max(0) as usize {
            list.set_column(
                index,
                &UiListColumn {
                    position: self.read_int(cursor, at)?,
                    width: self.read_int(cursor, at)?,
                    max_chars: self.read_int(cursor, at)?,
                },
            );
        }
        Ok(())
    }

    /// Parse a `cvarStrList` label/value list (donor `readStringChoices`).
    fn read_string_choices(
        &self,
        multi: &UiMultiDefinition,
        cursor: &mut UiMenuTokenCursor,
        at: &SourceLocation,
    ) -> Result<(), MenuError> {
        multi.set_count(0);
        multi.set_string_definition(true);
        self.expect_first(cursor, "{", at)?;
        let mut reading_label = true;
        loop {
            let token = self.expect_any(cursor, at, true)?;
            let value = token_value(&token);
            if value.starts_with('}') {
                return Ok(());
            }
            if value.starts_with(',') || value.starts_with(';') {
                continue;
            }
            if reading_label {
                multi.set_label(multi.count().max(0) as usize, self.allocate_string_reference(&value)?);
                reading_label = false;
            } else {
                multi.set_string_value(multi.count().max(0) as usize, self.allocate_string_reference(&value)?);
                reading_label = true;
                multi.set_count(multi.count() + 1);
                if multi.count() >= MULTI_SLOTS as i32 {
                    return Err(MenuError::Abort);
                }
            }
        }
    }

    /// Parse a `cvarFloatList` label/number list (donor `readNumberChoices`).
    fn read_number_choices(
        &self,
        multi: &UiMultiDefinition,
        cursor: &mut UiMenuTokenCursor,
        at: &SourceLocation,
    ) -> Result<(), MenuError> {
        multi.set_count(0);
        multi.set_string_definition(false);
        self.expect_first(cursor, "{", at)?;
        loop {
            let token = self.expect_any(cursor, at, true)?;
            let value = token_value(&token);
            if value.starts_with('}') {
                return Ok(());
            }
            if value.starts_with(',') || value.starts_with(';') {
                continue;
            }
            multi.set_label(multi.count().max(0) as usize, self.allocate_string_reference(&value)?);
            multi.set_number_value(
                multi.count().max(0) as usize,
                self.read_float(cursor, token.location())?,
            );
            multi.set_count(multi.count() + 1);
            if multi.count() >= MULTI_SLOTS as i32 {
                return Err(MenuError::Abort);
            }
        }
    }

    /// Apply full-screen and screen-coordinate fixups (donor `postParse`).
    fn post_parse(&self, menu: &UiMenuDefinition) {
        if menu.full_screen() != 0 {
            let rect = menu.window().rect();
            rect.set_x(0.0);
            rect.set_y(0.0);
            rect.set_width(640.0);
            rect.set_height(480.0);
        }
        let window = menu.window();
        let rect = window.rect();
        let mut x = rect.x();
        let mut y = rect.y();
        if window.border() != 0 {
            x += window.border_size();
            y += window.border_size();
        }
        for index in 0..menu.item_count().max(0) as usize {
            // Item_SetScreenCoords accepts a NULL member without touching it.
            let Some(item) = menu.slot_item(index) else {
                continue;
            };
            let item_window = item.window();
            let mut item_x = x;
            let mut item_y = y;
            if item_window.border() != 0 {
                item_x += item_window.border_size();
                item_y += item_window.border_size();
            }
            let client = item_window.client_rect();
            let screen = item_window.rect();
            screen.set_x(item_x + client.x());
            screen.set_y(item_y + client.y());
            screen.set_width(client.width());
            screen.set_height(client.height());
            let text = item.text_rect();
            text.set_width(0.0);
            text.set_height(0.0);
        }
    }

    /// Parse a font path plus point size (donor `readFont`).
    fn read_font(&self, cursor: &mut UiMenuTokenCursor, at: &SourceLocation) -> Result<UiFontReference, MenuError> {
        Ok(UiFontReference {
            path: self.read_string(cursor, at)?,
            point_size: self.read_int(cursor, at)?,
        })
    }

    /// Allocate a detached or pooled string (donor `allocateString`).
    fn allocate_string(&self, text: &str) -> Result<Option<String>, ClientError> {
        match &self.memory {
            UiMenuMemoryOwnership::Unaccounted => Ok(Some(UiStringReference::literal(text)?.read())),
            UiMenuMemoryOwnership::Qvm32 { .. } => self.storage.string_alloc(Some(text)),
        }
    }

    /// Allocate a detached or pooled string reference.
    fn allocate_string_reference(&self, text: &str) -> Result<Option<UiStringReference>, ClientError> {
        match &self.memory {
            UiMenuMemoryOwnership::Unaccounted => Ok(Some(UiStringReference::literal(text)?)),
            UiMenuMemoryOwnership::Qvm32 { .. } => self.storage.string_alloc_reference(Some(text)),
        }
    }

    /// Read one string token into a reference (donor `readStringReference`).
    fn read_string_reference(
        &self,
        cursor: &mut UiMenuTokenCursor,
        at: &SourceLocation,
    ) -> Result<Option<UiStringReference>, MenuError> {
        let token = self.expect_any(cursor, at, false)?;
        let value = token_value(&token);
        Ok(self.allocate_string_reference(&value)?)
    }

    /// Read one string token (donor `readString`).
    fn read_string(&self, cursor: &mut UiMenuTokenCursor, at: &SourceLocation) -> Result<Option<String>, MenuError> {
        let token = self.expect_any(cursor, at, false)?;
        let value = token_value(&token);
        Ok(self.allocate_string(&value)?)
    }

    /// Read one integer with optional leading minus (donor `readInt`).
    fn read_int(&self, cursor: &mut UiMenuTokenCursor, at: &SourceLocation) -> Result<i32, MenuError> {
        let mut token = self.expect_any(cursor, at, false)?;
        let mut negative = false;
        if token_value(&token).starts_with('-') {
            negative = true;
            let location = token.location().clone();
            token = self.expect_any(cursor, &location, false)?;
        }
        let ScriptToken::Number { integer_value, .. } = &token else {
            let location = token.location().clone();
            let value = token_value(&token);
            self.source_error(&format!("expected integer but found {value}"), &location, cursor);
            return Err(MenuError::Abort);
        };
        let stored = *integer_value as i32;
        Ok(if negative { stored.wrapping_neg() } else { stored })
    }

    /// Read one float with optional leading minus (donor `readFloat`).
    fn read_float(&self, cursor: &mut UiMenuTokenCursor, at: &SourceLocation) -> Result<f32, MenuError> {
        let mut token = self.expect_any(cursor, at, false)?;
        let mut negative = false;
        if token_value(&token).starts_with('-') {
            negative = true;
            let location = token.location().clone();
            token = self.expect_any(cursor, &location, false)?;
        }
        let ScriptToken::Number { float_value, .. } = &token else {
            let location = token.location().clone();
            let value = token_value(&token);
            self.source_error(&format!("expected float but found {value}"), &location, cursor);
            return Err(MenuError::Abort);
        };
        let stored = *float_value as f32;
        Ok(if negative { -stored } else { stored })
    }

    /// Read four floats into a rectangle (donor `readRectInto`).
    fn read_rect_into(
        &self,
        destination: &UiMutableRect,
        cursor: &mut UiMenuTokenCursor,
        at: &SourceLocation,
    ) -> Result<(), MenuError> {
        destination.set_x(self.read_float(cursor, at)?);
        destination.set_y(self.read_float(cursor, at)?);
        destination.set_width(self.read_float(cursor, at)?);
        destination.set_height(self.read_float(cursor, at)?);
        Ok(())
    }

    /// Read four floats into a color (donor `readColorInto`).
    fn read_color_into(
        &self,
        destination: &UiMutableColor,
        cursor: &mut UiMenuTokenCursor,
        at: &SourceLocation,
    ) -> Result<(), MenuError> {
        destination.set_x(self.read_float(cursor, at)?);
        destination.set_y(self.read_float(cursor, at)?);
        destination.set_z(self.read_float(cursor, at)?);
        destination.set_w(self.read_float(cursor, at)?);
        Ok(())
    }

    /// Read a foreground color, setting the flag per component (donor `readForeColor`).
    fn read_fore_color(
        &self,
        window: &UiWindowDefinition,
        cursor: &mut UiMenuTokenCursor,
        at: &SourceLocation,
    ) -> Result<(), MenuError> {
        for component in [
            UiColorComponent::X,
            UiColorComponent::Y,
            UiColorComponent::Z,
            UiColorComponent::W,
        ] {
            window
                .fore_color()
                .set_component(component, self.read_float(cursor, at)?);
            window.set_flags(window.flags() | UiWindowFlag::FORE_COLOR_SET);
        }
        Ok(())
    }

    /// Read four floats as a color value (donor `readColor`).
    fn read_color(&self, cursor: &mut UiMenuTokenCursor, at: &SourceLocation) -> Result<Vec4, MenuError> {
        Ok(Vec4 {
            x: self.read_float(cursor, at)?,
            y: self.read_float(cursor, at)?,
            z: self.read_float(cursor, at)?,
            w: self.read_float(cursor, at)?,
        })
    }

    /// Parse a braced script with 1023-byte truncation (donor `readScript`).
    fn read_script(&self, cursor: &mut UiMenuTokenCursor, at: &SourceLocation) -> Result<Option<UiScript>, MenuError> {
        self.expect(cursor, "{", at)?;
        let mut text = String::new();
        let mut truncated = false;
        let mut tokens = Vec::new();
        loop {
            let token = self.expect_any(cursor, at, false)?;
            let value = token_value(&token);
            if value == "}" {
                if truncated {
                    self.warn(&format!("script truncated to {MAX_UI_SCRIPT_BYTES} bytes"), at);
                }
                return match self.allocate_string_reference(&text)? {
                    Some(allocated) => Ok(Some(UiScript {
                        text: allocated.read(),
                        tokens,
                        truncated,
                    })),
                    None => Ok(None),
                };
            }
            if value.chars().any(|ch| ch as u32 > 255) {
                return Err(MenuError::Fatal(ClientError::BadUi(
                    "PC_Script_Parse requires an 8-bit token string".to_string(),
                )));
            }
            let rendered = if value.chars().count() > 1 {
                format!("\"{value}\"")
            } else {
                value.clone()
            };
            let addition = format!("{rendered} ");
            let remaining = MAX_UI_SCRIPT_BYTES - text.len();
            if remaining >= addition.len() {
                text.push_str(&addition);
                tokens.push(UiScriptToken {
                    text: value,
                    location: token.location().clone(),
                });
            } else {
                if remaining > 0 {
                    text.push_str(&addition[..remaining]);
                }
                truncated = true;
            }
        }
    }

    /// Expect an exact token, aborting on mismatch (donor `expect`).
    fn expect(&self, cursor: &mut UiMenuTokenCursor, expected: &str, at: &SourceLocation) -> Result<(), MenuError> {
        let token = self.expect_any(cursor, at, false)?;
        if token_value(&token).to_lowercase() != expected.to_lowercase() {
            return Err(MenuError::Abort);
        }
        Ok(())
    }

    /// Expect a token starting with `expected` (donor `expectFirst`).
    fn expect_first(
        &self,
        cursor: &mut UiMenuTokenCursor,
        expected: &str,
        at: &SourceLocation,
    ) -> Result<(), MenuError> {
        let token = self.expect_any(cursor, at, false)?;
        if !token_value(&token).to_lowercase().starts_with(&expected.to_lowercase()) {
            return Err(MenuError::Abort);
        }
        Ok(())
    }

    /// Read any token, reporting item-block EOF when asked (donor `expectAny`).
    fn expect_any(
        &self,
        cursor: &mut UiMenuTokenCursor,
        at: &SourceLocation,
        report_item_end: bool,
    ) -> Result<ScriptToken, MenuError> {
        match cursor.next_token()? {
            Some(token) => Ok(token),
            None => {
                if report_item_end {
                    self.source_error("end of file inside menu item", at, cursor);
                }
                Err(MenuError::Abort)
            }
        }
    }

    /// Open one file through the host (donor `openFile`).
    fn open_file(&mut self, path: &str) -> Option<UiMenuSourceInput> {
        match &mut self.host {
            UiMenuParseHost::Resolved(host) => host.resolver.resolve_root(path).map(UiMenuSourceInput::Source),
            UiMenuParseHost::Handle(host) => {
                UiMenuTokenCursor::open_handle(path, &host.sources, &host.assert_current_operation)
                    .map(UiMenuSourceInput::Cursor)
            }
        }
    }

    /// Resolve a required root, failing the load when missing (donor `resolveRoot`).
    fn resolve_root(&mut self, path: &str, label: &str) -> Result<UiMenuSourceInput, ClientError> {
        if path.is_empty() {
            return Err(self.fail(
                &format!("{label} path cannot be empty"),
                &SourceLocation {
                    path: "<ui>".to_string(),
                    line: 1,
                    column: 1,
                },
            ));
        }
        let source = match self.open_file(path) {
            Some(source) => source,
            None => {
                return Err(self.fail(
                    &format!("couldn't load {label} {path}"),
                    &SourceLocation {
                        path: path.to_string(),
                        line: 1,
                        column: 1,
                    },
                ));
            }
        };
        if source.path().is_empty() {
            return Err(self.fail(
                &format!("{label} resolver returned an empty canonical path"),
                &SourceLocation {
                    path: path.to_string(),
                    line: 1,
                    column: 1,
                },
            ));
        }
        Ok(source)
    }

    /// Publish one asset change, if a sink is installed.
    fn publish_asset(&mut self, event: UiMenuAssetPublication) {
        if let Some(sink) = self.asset_sink.as_mut() {
            sink.publish(&event);
        }
    }

    /// Record a diagnostic and notify the observer (donor `reportDiagnostic`).
    fn report_diagnostic(&self, diagnostic: ScriptDiagnostic) {
        let mut shared = self.reports.borrow_mut();
        shared.diagnostics.push(diagnostic.clone());
        if let Some(report) = shared.user_report.as_mut() {
            report(&diagnostic);
        }
    }

    /// Record an error at the cursor file/line plus the keyword column.
    ///
    /// The donor column quirk (`at.column` with the cursor file/line) is
    /// preserved verbatim.
    fn source_error(&self, message: &str, at: &SourceLocation, cursor: &mut UiMenuTokenCursor) {
        let position = cursor.position();
        self.report_diagnostic(ScriptDiagnostic {
            severity: DiagnosticSeverity::Error,
            message: message.to_string(),
            location: SourceLocation {
                path: position.filename,
                line: position.line,
                column: at.column,
            },
        });
    }

    /// Record a warning (donor `warn`).
    fn warn(&self, message: &str, at: &SourceLocation) {
        self.report_diagnostic(ScriptDiagnostic {
            severity: DiagnosticSeverity::Warning,
            message: message.to_string(),
            location: at.clone(),
        });
    }

    /// Record an error and build the fatal failure (donor `fail`).
    fn fail(&self, message: &str, at: &SourceLocation) -> ClientError {
        self.report_diagnostic(ScriptDiagnostic {
            severity: DiagnosticSeverity::Error,
            message: message.to_string(),
            location: at.clone(),
        });
        ClientError::BadUi(format!("{}:{}:{}: {message}", at.path, at.line, at.column))
    }
}

/// Token text with NUL truncation (donor `tokenValue`).
fn token_value(token: &ScriptToken) -> String {
    let base = if token.kind() == "string" {
        token.value().unwrap_or_else(|| token.text())
    } else {
        token.text()
    };
    match base.find('\0') {
        Some(end) => base[..end].to_string(),
        None => base.to_string(),
    }
}

/// Whether a type code carries edit-field type data (donor `isEditType`).
fn is_edit_type(item_type: i32) -> bool {
    item_type == UiItemTypeCode::Text as i32
        || item_type == UiItemTypeCode::EditField as i32
        || item_type == UiItemTypeCode::NumericField as i32
        || item_type == UiItemTypeCode::Slider as i32
        || item_type == UiItemTypeCode::YesNo as i32
        || item_type == UiItemTypeCode::Bind as i32
}

/// `COM_Parse` token storage bound.
const COM_MAX_TOKEN_CHARS: usize = 1024;

/// Signed-byte view of a Latin-1 byte (donor `signedByte` body).
fn com_signed(byte: u8) -> i32 {
    let value = i32::from(byte);
    if value >= 128 {
        value - 256
    } else {
        value
    }
}

/// Byte at `offset`, or NUL past the end (terminated profile).
fn com_at(data: &[u8], offset: usize) -> i32 {
    match data.get(offset) {
        Some(byte) => com_signed(*byte),
        None => 0,
    }
}

/// `COM_Compress`: strip comments, coalesce whitespace (donor `compressCommonText`).
fn compress_common_text(source: &str) -> Result<Vec<u8>, ClientError> {
    let mut bytes = Vec::with_capacity(source.len());
    for (index, ch) in source.chars().enumerate() {
        let byte = ch as u32;
        if byte > 255 {
            return Err(ClientError::BadUi(format!(
                "COM_Parse source is not a Latin-1 byte string at {index}"
            )));
        }
        bytes.push(byte as u8);
    }
    let mut offset = 0;
    let mut output = Vec::new();
    let mut newline = false;
    let mut whitespace = false;
    while com_at(&bytes, offset) != 0 {
        let byte = com_at(&bytes, offset);
        if byte == 47 && com_at(&bytes, offset + 1) == 47 {
            while com_at(&bytes, offset) != 0 && com_at(&bytes, offset) != 10 {
                offset += 1;
            }
        } else if byte == 47 && com_at(&bytes, offset + 1) == 42 {
            while com_at(&bytes, offset) != 0 && (com_at(&bytes, offset) != 42 || com_at(&bytes, offset + 1) != 47) {
                offset += 1;
            }
            if com_at(&bytes, offset) != 0 {
                offset += 2;
            }
        } else if byte == 10 || byte == 13 {
            newline = true;
            offset += 1;
        } else if byte == 32 || byte == 9 {
            whitespace = true;
            offset += 1;
        } else {
            if newline {
                output.push(b'\n');
                newline = false;
                whitespace = false;
            }
            if whitespace {
                output.push(b' ');
                whitespace = false;
            }
            output.push(bytes[offset]);
            offset += 1;
            if byte == 34 {
                while com_at(&bytes, offset) != 0 && com_at(&bytes, offset) != 34 {
                    output.push(bytes[offset]);
                    offset += 1;
                }
                if com_at(&bytes, offset) == 34 {
                    output.push(bytes[offset]);
                    offset += 1;
                }
            }
        }
    }
    Ok(output)
}

/// One `COM_Parse` token; empty means end of source (donor `parse`).
fn com_parse_token(data: &[u8], offset: &mut Option<usize>) -> Result<Vec<u8>, ClientError> {
    let mut token = Vec::new();
    let Some(mut pos) = *offset else {
        return Ok(token);
    };
    loop {
        let mut c;
        loop {
            c = com_at(data, pos);
            if c == 0 {
                *offset = None;
                return Ok(token);
            }
            if c <= 32 {
                pos += 1;
            } else {
                break;
            }
        }
        if c == 47 && com_at(data, pos + 1) == 47 {
            pos += 2;
            while com_at(data, pos) != 0 && com_at(data, pos) != 10 {
                pos += 1;
            }
        } else if c == 47 && com_at(data, pos + 1) == 42 {
            pos += 2;
            while com_at(data, pos) != 0 && (com_at(data, pos) != 42 || com_at(data, pos + 1) != 47) {
                pos += 1;
            }
            if com_at(data, pos) != 0 {
                pos += 2;
            }
        } else {
            break;
        }
    }
    let c = com_at(data, pos);
    if c == 34 {
        pos += 1;
        loop {
            let c = com_at(data, pos);
            pos += 1;
            if c == 34 || c == 0 {
                if token.len() == COM_MAX_TOKEN_CHARS {
                    return Err(ClientError::BadUi(
                        "COM_Parse quoted token terminator exceeds 1024-byte storage".to_string(),
                    ));
                }
                *offset = if c == 0 { None } else { Some(pos) };
                return Ok(token);
            }
            if token.len() < COM_MAX_TOKEN_CHARS {
                token.push(data[pos - 1]);
            }
        }
    }
    loop {
        if token.len() < COM_MAX_TOKEN_CHARS {
            token.push(data[pos]);
        }
        pos += 1;
        if com_at(data, pos) <= 32 {
            break;
        }
    }
    if token.len() == COM_MAX_TOKEN_CHARS {
        token.clear();
    }
    *offset = Some(pos);
    Ok(token)
}

/// Decode Latin-1 bytes.
fn latin1_to_string(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| char::from(*byte)).collect()
}

/// Load menu definitions for one plan (donor `loadMenuDefinitions`).
///
/// `Promise -> sync`: the donor is async; this port loads synchronously.
pub fn load_menu_definitions(
    host: UiMenuParseHost,
    plan: UiMenuLoadPlan,
    preprocessor_options: ScriptPreprocessorOptions,
    parse_options: UiMenuParseOptions,
) -> Result<UiMenuDefinitions, ClientError> {
    UiMenuSourceParser::new(host, preprocessor_options, parse_options).load(&plan)
}
#[cfg(test)]
mod tests {
    use super::*;

    struct MapResolver {
        files: HashMap<String, String>,
    }

    impl IncludeResolver for MapResolver {
        fn resolve(&mut self, request: &IncludeRequest) -> Result<Option<ScriptSource>, ClientError> {
            Ok(self.files.get(&request.requested_path).map(|text| ScriptSource {
                path: request.requested_path.clone(),
                text: text.clone(),
            }))
        }
    }

    impl UiMenuResolver for MapResolver {
        fn resolve_root(&mut self, path: &str) -> Option<ScriptSource> {
            self.files.get(path).map(|text| ScriptSource {
                path: path.to_string(),
                text: text.clone(),
            })
        }
    }

    struct FixedRandom(i32);

    impl UiMenuRandom for FixedRandom {
        fn next_int(&mut self) -> i32 {
            self.0
        }
    }

    struct VecRegSink {
        events: Vec<UiMenuRegistrationEvent>,
        next_handle: i32,
    }

    impl UiMenuRegistrationSink for VecRegSink {
        fn register(&mut self, event: &UiMenuRegistrationEvent) -> Result<UiMenuRegistrationResult, ClientError> {
            self.events.push(event.clone());
            let handle = self.next_handle;
            self.next_handle += 1;
            Ok(Some(UiMenuRegistrationHandle { handle }))
        }
    }

    struct VecAssetSink {
        events: Vec<UiMenuAssetPublication>,
    }

    impl UiMenuAssetSink for VecAssetSink {
        fn publish(&mut self, event: &UiMenuAssetPublication) {
            self.events.push(event.clone());
        }
    }

    struct VecMenuSink {
        menus: Vec<UiMenuDefinition>,
    }

    impl UiMenuDefinitionSink for VecMenuSink {
        fn menu_count(&self) -> usize {
            self.menus.len()
        }

        fn publish(&mut self, menu: &UiMenuDefinition) -> Result<(), ClientError> {
            self.menus.push(menu.clone());
            Ok(())
        }
    }

    fn test_host(files: HashMap<String, String>) -> UiMenuParseHost {
        UiMenuParseHost::Resolved(UiMenuResolvedParseHost::new(
            SharedUiMenuResolver::new(MapResolver { files }),
            FixedRandom(42),
        ))
    }

    fn files(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(path, text)| (path.to_string(), text.to_string()))
            .collect()
    }

    fn location() -> SourceLocation {
        SourceLocation {
            path: "test".to_string(),
            line: 1,
            column: 1,
        }
    }

    #[test]
    fn constants_match_donor() {
        assert_eq!(MAX_UI_MENUS, 64);
        assert_eq!(MAX_UI_MENU_ITEMS, 96);
        assert_eq!(MAX_UI_COLOR_RANGES, 10);
        assert_eq!(MAX_UI_LIST_COLUMNS, 16);
        assert_eq!(MAX_UI_MULTI_CHOICES, 31);
        assert_eq!(MAX_UI_SCRIPT_BYTES, 1023);
        assert_eq!(MAX_HUD_MENU_SET_BYTES, 4095);
        assert_eq!(UiWindowFlag::VISIBLE, 0x0000_0004);
        assert_eq!(UiWindowFlag::FORE_COLOR_SET, 0x0000_0200);
        assert_eq!(UiWindowFlag::TIMED_VISIBLE, 0x0080_0000);
        assert_eq!(UiItemTypeCode::Bind as i32, 13);
        assert_eq!(UiItemTypeCode::from_i32(6), Some(UiItemTypeCode::ListBox));
        assert_eq!(UiItemTypeCode::from_i32(99), None);
    }

    #[test]
    fn string_reference_round_trip() {
        let reference = UiStringReference::literal("hello").unwrap();
        assert_eq!(reference.read(), "hello");
        let truncated = UiStringReference::literal("ab\0cd").unwrap();
        assert_eq!(truncated.read(), "ab");
        assert!(UiStringReference::literal("héllo→").is_err());
    }

    #[test]
    fn window_wrapper_uses_donor_offsets() {
        let alloc = UiMemoryAllocation::zeroed(WINDOW_SIZE);
        let window = UiWindowDefinition::new(alloc.clone());
        window.initialize();
        assert_eq!(window.cinematic_handle(), -1);
        assert_eq!(window.border_size(), 1.0);
        assert_eq!(
            window.fore_color().snapshot(),
            Vec4 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
                w: 1.0,
            }
        );
        window.set_flags(UiWindowFlag::VISIBLE | UiWindowFlag::POPUP);
        assert_eq!(alloc.get_i32(WINDOW_FLAGS), 0x0020_0004);
        window.set_rect(&UiRect {
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
        });
        assert_eq!(alloc.get_f32(WINDOW_RECT), 1.0);
        assert_eq!(alloc.get_f32(WINDOW_RECT + 12), 4.0);
        window.set_border_size(2.5);
        assert_eq!(
            f32::from_le_bytes([
                alloc.arena.borrow().bytes[WINDOW_BORDER_SIZE],
                alloc.arena.borrow().bytes[WINDOW_BORDER_SIZE + 1],
                alloc.arena.borrow().bytes[WINDOW_BORDER_SIZE + 2],
                alloc.arena.borrow().bytes[WINDOW_BORDER_SIZE + 3],
            ]),
            2.5
        );
        window.set_background(
            UiShaderReference {
                path: Some("bg".to_string()),
            },
            Some(7),
        );
        assert_eq!(window.background_handle(), Some(7));
        assert_eq!(window.background().unwrap().path, Some("bg".to_string()));
    }

    #[test]
    fn item_wrapper_uses_donor_offsets() {
        let alloc = UiMemoryAllocation::zeroed(ITEM_SIZE);
        let item = UiItemDefinition::new(alloc.clone(), location(), Some(128));
        item.initialize();
        assert_eq!(item.allocation_offset(), Some(128));
        assert_eq!(item.text_scale(), 0.55);
        item.set_item_type(UiItemTypeCode::Slider as i32);
        assert_eq!(alloc.get_i32(ITEM_TYPE), 10);
        item.set_text_align_x(1.5);
        assert_eq!(alloc.get_f32(ITEM_TEXT_ALIGN_X), 1.5);
        item.set_cvar_rule(Some(UiCvarRule::Show { script: None }));
        assert_eq!(alloc.get_i32(ITEM_CVAR_FLAGS), 4);
        assert_eq!(item.cvar_rule().unwrap().kind(), "show");
        item.set_cvar_rule(None);
        assert_eq!(item.cvar_flags(), 0);
        assert!(item.cvar_rule().is_none());
        item.add_color_range(&UiColorRange {
            low: 0.25,
            high: 0.75,
            color: Vec4 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
                w: 1.0,
            },
        });
        assert_eq!(alloc.get_f32(ITEM_COLOR_RANGES + 16), 0.25);
        assert_eq!(item.color_ranges().unwrap().len(), 1);
        item.set_special(3.0);
        item.set_cursor_position(9);
        assert_eq!(alloc.get_f32(ITEM_SPECIAL), 3.0);
        assert_eq!(alloc.get_i32(ITEM_CURSOR), 9);
        assert!(!item.has_type_data());
        item.set_type_data(Some(UiMemoryAllocation::zeroed(EDIT_SIZE)));
        assert!(item.has_type_data());
        assert!(matches!(item.behavior(), UiItemBehavior::Slider { .. }));
        item.set_item_type(99);
        assert!(matches!(item.behavior(), UiItemBehavior::Unknown { type_code: 99 }));
    }

    #[test]
    fn color_ranges_rejects_overflow() {
        let item = UiItemDefinition::new(UiMemoryAllocation::zeroed(ITEM_SIZE), location(), None);
        item.set_color_count(11);
        assert!(item.color_ranges().is_err());
        item.set_color_count(-2);
        assert!(item.color_ranges().unwrap().is_empty());
    }

    #[test]
    fn multi_list_wrappers_use_donor_offsets() {
        let multi = UiMultiDefinition::new(UiMemoryAllocation::zeroed(MULTI_SIZE));
        multi.set_count(2);
        multi.set_string_definition(true);
        multi.set_label(1, Some(UiStringReference::literal("L1").unwrap()));
        multi.set_string_value(1, Some(UiStringReference::literal("V1").unwrap()));
        multi.set_number_value(1, 2.5);
        assert_eq!(multi.label(1).as_deref(), Some("L1"));
        assert_eq!(multi.string_value(1).as_deref(), Some("V1"));
        assert_eq!(multi.number_value(1), 2.5);

        let list = UiListBoxDefinition::new(UiMemoryAllocation::zeroed(LIST_SIZE));
        list.set_column_count(2);
        list.set_column(
            1,
            &UiListColumn {
                position: 4,
                width: 8,
                max_chars: 12,
            },
        );
        let columns = list.columns().unwrap();
        assert_eq!(columns.len(), 2);
        assert_eq!(columns[1].width, 8);
        list.set_column_count(17);
        assert!(list.columns().is_err());
    }

    #[test]
    fn menu_wrapper_slots_and_items() {
        let memory = MenuArenaMemory::new();
        let menu = UiMenuDefinition::new(memory.menu_record(3), location());
        assert_eq!(menu.source_index(), 3);
        menu.initialize(&UiGlobalAssets::default());
        assert_eq!(menu.cursor_item(), -1);
        let item = UiItemDefinition::new(UiMemoryAllocation::zeroed(ITEM_SIZE), location(), None);
        menu.set_item(0, Some(item));
        menu.set_item_count(1);
        assert_eq!(menu.items().unwrap().len(), 1);
        assert!(menu.item_at(1).is_none());
        menu.set_item_count(97);
        assert!(menu.items().is_err());
        menu.set_item_count(1);
        menu.set_item(0, None);
        assert!(menu.items().is_err());
    }

    #[test]
    fn rect_conversions_cover_draw_rect() {
        let rect = UiRect {
            x: 1.0,
            y: 2.0,
            width: 3.0,
            height: 4.0,
        };
        let draw = Rect::from(&rect);
        assert_eq!((draw.x, draw.y, draw.width, draw.height), (1.0, 2.0, 3.0, 4.0));
        assert_eq!(UiRect::from(&draw), rect);
    }

    const MAIN_MENU: &str = r#"
assetGlobalDef {
  font "fonts/font1" 48
  fadeClamp 0.5
}
menuDef {
  name "main"
  rect 0 0 640 480
  visible 1
  itemDef {
    name "title"
    text "Hello"
    rect -4 2 100 30
    type 0
    textscale 0.5
    forecolor 1 1 1 1
    action { play sound }
  }
}
"#;

    #[test]
    fn parses_valid_menu_set() {
        let host = test_host(files(&[
            ("ui/menus.txt", "loadmenu { \"ui/main.menu\" }"),
            ("ui/main.menu", MAIN_MENU),
        ]));
        let definitions = load_menu_definitions(
            host,
            UiMenuLoadPlan::Ui {
                set_paths: vec!["ui/menus.txt".to_string()],
            },
            ScriptPreprocessorOptions::default(),
            UiMenuParseOptions::default(),
        )
        .unwrap();
        assert_eq!(definitions.menus.len(), 1);
        assert_eq!(
            definitions.loaded_files,
            vec!["ui/menus.txt".to_string(), "ui/main.menu".to_string()]
        );
        assert!(definitions.diagnostics.is_empty());
        assert!(definitions.font_registered);
        assert_eq!(definitions.assets.fade_clamp, 0.5);
        assert_eq!(
            definitions.assets.text_font.as_ref().unwrap().path,
            Some("fonts/font1".to_string())
        );
        assert_eq!(definitions.registration.kind(), "deferred");
        assert_eq!(definitions.registration.events().len(), 1);

        let menu = &definitions.menus[0];
        assert_eq!(menu.window().name().as_deref(), Some("main"));
        assert_eq!(menu.item_count(), 1);
        let items = menu.items().unwrap();
        assert_eq!(items[0].text().as_deref(), Some("Hello"));
        assert_eq!(items[0].text_scale(), 0.5);
        // Negative rect x exercises the leading-minus number path.
        assert_eq!(items[0].window().client_rect().x(), -4.0);
        // Screen coords equal client coords for a borderless menu at origin.
        assert_eq!(items[0].window().rect().x(), -4.0);
        assert_eq!(
            items[0].window().flags() & UiWindowFlag::FORE_COLOR_SET,
            UiWindowFlag::FORE_COLOR_SET
        );
        let action = items[0].action().unwrap();
        assert_eq!(action.text, "\"play\" \"sound\" ");
        assert_eq!(action.tokens.len(), 2);
        assert!(!action.truncated);
        assert!(items[0].parent().is_some());
    }

    #[test]
    fn unknown_menu_keyword_reports_and_keeps_menu() {
        let host = test_host(files(&[
            ("ui/menus.txt", "loadmenu { \"ui/bad.menu\" }"),
            ("ui/bad.menu", "menuDef { name \"x\" boguskeyword }"),
        ]));
        let definitions = load_menu_definitions(
            host,
            UiMenuLoadPlan::Ui {
                set_paths: vec!["ui/menus.txt".to_string()],
            },
            ScriptPreprocessorOptions::default(),
            UiMenuParseOptions::default(),
        )
        .unwrap();
        assert_eq!(definitions.menus.len(), 1);
        assert_eq!(definitions.diagnostics.len(), 1);
        assert!(definitions.diagnostics[0]
            .message
            .contains("unknown menu keyword boguskeyword"));
    }

    #[test]
    fn truncated_menu_field_skips_menu() {
        let host = test_host(files(&[
            ("ui/menus.txt", "loadmenu { \"ui/bad.menu\" }"),
            ("ui/bad.menu", "menuDef { name \"x\" rect 1"),
        ]));
        let definitions = load_menu_definitions(
            host,
            UiMenuLoadPlan::Ui {
                set_paths: vec!["ui/menus.txt".to_string()],
            },
            ScriptPreprocessorOptions::default(),
            UiMenuParseOptions::default(),
        )
        .unwrap();
        assert!(definitions.menus.is_empty());
        assert!(definitions
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("couldn't parse menu keyword")));
    }

    #[test]
    fn missing_set_file_is_fatal() {
        let host = test_host(files(&[]));
        let error = load_menu_definitions(
            host,
            default_ui_menu_plan(),
            ScriptPreprocessorOptions::default(),
            UiMenuParseOptions::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("couldn't load menu set"));
    }

    #[test]
    fn parses_hud_set_without_font_registration() {
        let host = test_host(files(&[
            ("ui/hud.txt", "// comment\nloadmenu { \"ui/hudmain.menu\" }"),
            ("ui/hudmain.menu", MAIN_MENU),
        ]));
        let definitions = load_menu_definitions(
            host,
            default_hud_menu_plan(),
            ScriptPreprocessorOptions::default(),
            UiMenuParseOptions::default(),
        )
        .unwrap();
        assert_eq!(definitions.menus.len(), 1);
        assert!(!definitions.font_registered);
        assert_eq!(
            definitions.loaded_files,
            vec!["ui/hud.txt".to_string(), "ui/hudmain.menu".to_string()]
        );
    }

    #[test]
    fn oversize_hud_set_is_fatal() {
        let big = format!("loadmenu {{ {}}}", "x".repeat(MAX_HUD_MENU_SET_BYTES));
        let host = test_host(files(&[("ui/hud.txt", &big)]));
        let error = load_menu_definitions(
            host,
            default_hud_menu_plan(),
            ScriptPreprocessorOptions::default(),
            UiMenuParseOptions::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("menu file too large"));
    }

    #[test]
    fn long_script_truncates_with_warning() {
        let words: Vec<String> = (0..300).map(|index| format!("w{index}")).collect();
        let menu = format!(
            "menuDef {{ name \"t\" itemDef {{ name \"i\" action {{ {} }} }} }}",
            words.join(" ")
        );
        let host = test_host(files(&[("ui/m.menu", &menu)]));
        let mut parser = UiMenuSourceParser::new(
            host,
            ScriptPreprocessorOptions::default(),
            UiMenuParseOptions::default(),
        );
        parser
            .parse_source(
                UiMenuSourceInput::Source(ScriptSource {
                    path: "ui/m.menu".to_string(),
                    text: menu.clone(),
                }),
                false,
            )
            .unwrap();
        assert_eq!(parser.menus.len(), 1);
        let action = parser.menus[0].items().unwrap()[0].action().unwrap();
        assert!(action.truncated);
        assert!(action.text.len() <= MAX_UI_SCRIPT_BYTES);
        assert!(parser
            .reports
            .borrow()
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("truncated")));
    }

    #[test]
    fn parses_multi_choices_and_clamped_columns() {
        let columns: Vec<String> = (0..48).map(|index| index.to_string()).collect();
        let menu = format!(
            r#"
menuDef {{
  name "m"
  itemDef {{
    name "c"
    type 12
    cvarStrList {{ "Low" ; "0" ; "High" ; "1" ; }}
  }}
  itemDef {{
    name "l"
    type 6
    columns 20 {}
  }}
}}
"#,
            columns.join(" ")
        );
        let host = test_host(files(&[("ui/m.menu", &menu)]));
        let mut parser = UiMenuSourceParser::new(
            host,
            ScriptPreprocessorOptions::default(),
            UiMenuParseOptions::default(),
        );
        parser
            .parse_source(
                UiMenuSourceInput::Source(ScriptSource {
                    path: "ui/m.menu".to_string(),
                    text: menu,
                }),
                false,
            )
            .unwrap();
        let items = parser.menus[0].items().unwrap();
        let multi = items[0].multi_data().unwrap();
        assert_eq!(multi.count(), 2);
        assert!(multi.string_definition());
        assert_eq!(multi.label(0).as_deref(), Some("Low"));
        assert_eq!(multi.string_value(1).as_deref(), Some("1"));
        let list = items[1].list_data().unwrap();
        assert_eq!(list.columns().unwrap().len(), 16);
    }

    #[test]
    fn sinks_observe_registrations_assets_and_menus() {
        let host = test_host(files(&[
            ("ui/menus.txt", "loadmenu { \"ui/main.menu\" }"),
            ("ui/main.menu", MAIN_MENU),
        ]));
        let mut parser = UiMenuSourceParser::new(
            host,
            ScriptPreprocessorOptions::default(),
            UiMenuParseOptions {
                registration_sink: Some(Box::new(VecRegSink {
                    events: Vec::new(),
                    next_handle: 10,
                })),
                asset_sink: Some(Box::new(VecAssetSink { events: Vec::new() })),
                ..UiMenuParseOptions::default()
            },
        );
        let definitions = parser
            .load(&UiMenuLoadPlan::Ui {
                set_paths: vec!["ui/menus.txt".to_string()],
            })
            .unwrap();
        assert_eq!(definitions.registration.kind(), "completed");

        let host = test_host(files(&[
            ("ui/menus.txt", "loadmenu { \"ui/main.menu\" }"),
            ("ui/main.menu", MAIN_MENU),
        ]));
        let mut streaming = UiMenuSourceParser::new(
            host,
            ScriptPreprocessorOptions::default(),
            UiMenuParseOptions {
                menu_sink: Some(Box::new(VecMenuSink { menus: Vec::new() })),
                ..UiMenuParseOptions::default()
            },
        );
        let streamed = streaming
            .load(&UiMenuLoadPlan::Ui {
                set_paths: vec!["ui/menus.txt".to_string()],
            })
            .unwrap();
        assert!(streamed.menus.is_empty());
        assert_eq!(streamed.loaded_files, vec!["ui/menus.txt".to_string()]);
    }

    #[test]
    fn out_of_memory_fails_item_init() {
        let memory = MenuArenaMemory::with_pool_sizes(64, 64);
        let host = test_host(files(&[
            ("ui/menus.txt", "loadmenu { \"ui/main.menu\" }"),
            ("ui/main.menu", MAIN_MENU),
        ]));
        let error = load_menu_definitions(
            host,
            UiMenuLoadPlan::Ui {
                set_paths: vec!["ui/menus.txt".to_string()],
            },
            ScriptPreprocessorOptions::default(),
            UiMenuParseOptions {
                memory: Some(UiMenuMemoryOwnership::Qvm32 {
                    memory: SharedUiMenuMemory::new(memory),
                }),
                ..UiMenuParseOptions::default()
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("Item_Init"));
    }

    #[test]
    fn cursor_opens_sources_and_missing_handles() {
        let resolver = SharedUiMenuResolver::new(MapResolver { files: HashMap::new() });
        let mut cursor = UiMenuTokenCursor::open(
            ScriptSource {
                path: "empty.menu".to_string(),
                text: String::new(),
            },
            resolver,
            ScriptPreprocessorOptions::default(),
        )
        .unwrap();
        assert_eq!(cursor.path(), "empty.menu");
        assert!(cursor.next_token().unwrap().is_none());
        cursor.dispose();

        struct Missing;
        impl UiMenuScriptSources for Missing {
            fn load_source_handle(&mut self, _path: &str) -> i32 {
                0
            }
            fn read_token_handle(&mut self, _handle: i32) -> Option<ScriptTokenRecord> {
                None
            }
            fn source_file_and_line(&mut self, _handle: i32) -> Option<ScriptSourcePosition> {
                None
            }
            fn free_source_handle(&mut self, _handle: i32) -> bool {
                false
            }
        }
        let sources = SharedUiMenuScriptSources::new(Missing);
        let guard: UiMenuOperationGuard = Rc::new(|| {});
        assert!(UiMenuTokenCursor::open_handle("nope.menu", &sources, &guard).is_none());
    }

    #[test]
    fn publication_and_plan_kinds_match_donor() {
        assert_eq!(default_ui_menu_plan().kind(), "ui");
        assert_eq!(default_hud_menu_plan().kind(), "hud");
        assert_eq!(UiMenuAssetPublication::FontRegistered(true).field(), "fontRegistered");
        assert_eq!(
            UiMenuAssetPublication::ShadowColorComponent {
                component: UiColorComponent::W,
                value: 1.0,
            }
            .field(),
            "shadowColorComponent"
        );
        assert_eq!(
            UiMenuRegistrationState::Deferred { events: Vec::new() }.kind(),
            "deferred"
        );
        assert_eq!(
            UiMenuRegistrationEvent::Sound {
                reference: UiSoundReference { path: None },
                location: location(),
            }
            .kind(),
            "sound"
        );
    }
}
