//! Team Arena shared-menu runtime (`ui_shared.c`).
//!
//! Donor provenance: `src/ui/common/legacy/runtime.ts` (translated from id
//! Software's `code/ui/ui_shared.c` and `ui_shared.h`). This is a full,
//! synchronous, idiomatic Rust port: every donor export and every
//! [`UiRuntime`] method is present under Rust case conventions.
//!
//! ## Sync policy
//!
//! The donor is `async` throughout; this port is fully synchronous. Each
//! former suspension point is documented on the affected trait or method as
//! `Promise -> sync`. Host traits take `&mut self` (recording fakes stay
//! plain structs) except [`UiCvarRegistry::get`], which is `&self` so
//! [`UiRuntime::snapshot`] can stay read-only like the donor.
//!
//! ## Float policy
//!
//! Layout and color math is `f32`, preserving the donor's `Math.fround`
//! operation order. `f64` appears only for the donor's transcendental
//! intermediates (`Math.sin`/`Math.cos` in the orbit and pulse paths),
//! rounded back to `f32` at the donor's `f(...)` boundaries. Integer-domain
//! values (time, counts, handles) are `i32`/`usize`.
//!
//! ## Ownership deviations (donor uses aliasing object graphs)
//!
//! - `MenuState`/`ItemState`/`WindowState` proxy layers collapse: the runtime
//!   owns [`UiMenuDefinition`] values (with owned item vectors) and refers to
//!   items by [`ItemState`] index handles. Observable behavior is unchanged.
//! - Captured-menu handles ([`UiCapturedMenu`]) are menu-slot indices instead
//!   of object-identity map keys; slots are preserved across repopulation
//!   exactly like the donor's static array.
//! - The item-allocation arena (`allocationOffset`/`itemArena`) is dropped;
//!   [`UiItemDefinition::allocation_offset`] is retained as data for
//!   definition fidelity but is not consulted.
//! - Lazy `PictureAsset | (() => PictureAsset)` backgrounds are resolved
//!   eagerly at paint time; [`Draw2D`] takes owned pictures and no host
//!   mutation interleaves within a paint call, so output is identical.
//! - Item and window handles are the shared-memory views from [`super::menu`],
//!   so mutation flows through getters/setters instead of field writes.
//!
//! Menu, script-token, and memory types come from the sibling modules
//! (`super::menu`, `super::script::preprocessor`, `super::team_arena`);
//! only runtime-owned hosts, options, and snapshots are defined here.
//!
//! All failures surface as [`ClientError::BadUi`].

use std::cell::Cell;
use std::collections::HashMap;

use qa_core::cmd::{ascii_fold, source_command_text};
use qa_core::identity::{ClientId, SeatId, SessionId};
use qa_core::math::Vec4;
use qa_core::numeric::qvm_float_to_int;

use super::borders::{draw_cg_rect, draw_cg_sides, draw_cg_top_bottom};
use super::menu::{
    UiColorComponent, UiGlobalAssets, UiItemBehavior, UiItemDefinition, UiItemTypeCode, UiListBoxDefinition,
    UiMenuAssetPublication, UiMenuDefinition, UiMenuDefinitions, UiMenuMemory, UiMenuMemoryOwnership,
    UiMenuRegistrationEvent, UiMenuRegistrationState, UiMenuResource, UiRect, UiScript, UiScriptToken,
    UiShaderReference, UiWindowDefinition, UiWindowFlag,
};
use super::script::preprocessor::SourceLocation;
use crate::input::{KeyCode, KEY_CHAR_FLAG};
use crate::text::draw2d::{Draw2D, PictureAsset, Rect};
use crate::text::q3_font::{
    text_height, text_paint, text_paint_with_cursor, text_width, FontSet, TextCursor, TextPaintOptions,
};
use crate::ClientError;

/// Maximum menus in the source static array (`MAX_UI_MENUS`).
pub const MAX_UI_MENUS: usize = 64;

/// Maximum entries pushed onto the open-menu stack (`MAX_OPEN_MENUS`).
const MAX_OPEN_MENUS: usize = 16;

/// Scrollbar control size in 640x480 units (`SCROLLBAR_SIZE`).
const SCROLLBAR_SIZE: f32 = 16.0;

/// Slider travel width in 640x480 units (`SLIDER_WIDTH`).
const SLIDER_WIDTH: f32 = 96.0;

/// Slider thumb width in 640x480 units (`SLIDER_THUMB_WIDTH`).
const SLIDER_THUMB_WIDTH: f32 = 12.0;

/// Double-click window in milliseconds (`DOUBLE_CLICK_DELAY`).
const DOUBLE_CLICK_DELAY: i32 = 300;

/// Focus-pulse time divisor (`PULSE_DIVISOR`).
const PULSE_DIVISOR: i32 = 75;

/// Text-blink time divisor (`BLINK_DIVISOR`).
const BLINK_DIVISOR: i32 = 200;

/// Script text truncation length (`MAX_UI_SCRIPT_BYTES`).
const MAX_SCRIPT_BYTES: usize = 1023;

/// `COM_Parse` token storage (`MAX_TOKEN_CHARS`).
const MAX_TOKEN_CHARS: usize = 1024;

/// UI edit-field backing buffer (`char[1024]`).
const EDIT_BUFFER_LEN: usize = 1024;

/// `Com_sprintf` destination bound (`BIG_BUFFER_BYTES`).
const BIG_BUFFER_BYTES: usize = 32_000;

/// `AddFloat` digit buffer bound.
const FLOAT_PRECISION_MAX: i32 = 32;

/// Missing-binding sentinel.
const NO_BINDING: i32 = -1;

/// Identity marker for a donor `Math.fround` boundary (see module docs).
#[inline(always)]
fn f(value: f32) -> f32 {
    value
}

/// Build a [`ClientError::BadUi`] message.
fn bad_ui(message: impl Into<String>) -> ClientError {
    ClientError::BadUi(message.into())
}

/// Truncate at NUL and reject non-byte text (`quakeString`).
fn quake_string(value: &str, location: Option<&SourceLocation>) -> Result<String, ClientError> {
    let text = value.split('\0').next().unwrap_or("");
    if text.chars().any(|c| c as u32 > 255) {
        let detail = match location {
            Some(found) => format!(" at {}:{}:{}", found.path, found.line, found.column),
            None => String::new(),
        };
        return Err(bad_ui(format!("Quake UI text must be an 8-bit string{detail}")));
    }
    Ok(text.to_string())
}

/// ASCII case-insensitive name comparison (`equalName`).
fn equal_name(left: Option<&str>, right: &str) -> bool {
    match left {
        Some(name) => ascii_fold(name) == ascii_fold(right),
        None => false,
    }
}

/// Resource-map key (`resourceKey`).
fn resource_key(path: Option<&str>) -> String {
    match path {
        None => "null".to_string(),
        Some(name) => format!("name:{}", ascii_fold(name.split('\0').next().unwrap_or(""))),
    }
}

/// Hit test with donor edge semantics (`rectContains`).
///
/// Edges are exclusive and the far edges round through `f32`, matching
/// `x > rect.x && x < f(rect.x + rect.width) && ...`.
fn rect_contains(rect: &UiRect, x: f32, y: f32) -> bool {
    x > rect.x && x < f(rect.x + rect.width) && y > rect.y && y < f(rect.y + rect.height)
}

/// Whether a key is a mouse button (`isMouseKey`).
fn is_mouse_key(key: i32) -> bool {
    key == KeyCode::Mouse1 as i32 || key == KeyCode::Mouse2 as i32 || key == KeyCode::Mouse3 as i32
}

/// Whether a key activates the focused control (`isActivateKey`).
fn is_activate_key(key: i32) -> bool {
    is_mouse_key(key) || key == KeyCode::Enter as i32
}

/// A `bg_lib` format argument (`GameFormatArgument`).
#[derive(Debug, Clone, PartialEq)]
enum GameFormatArg {
    /// Signed 32-bit integer (`%d`/`%i`/`%c`).
    Int(i32),
    /// Binary32 float (`%f`).
    Float(f32),
    /// Byte string, `None` for null (`%s`).
    Text(Option<String>),
}

/// Format byte strings with the QVM `bg_lib.c` rules (`gameFormat`).
///
/// Supports `%d`/`%i`/`%f`/`%s`/`%%` with `-`/width/precision and the
/// donor's byte-valued fallback for other specifiers, then applies
/// `Q_strncpyz` bounds.
fn game_format(format: &str, args: &[GameFormatArg]) -> Result<String, ClientError> {
    let bytes: Vec<u8> = {
        let mut out = Vec::with_capacity(format.len());
        for (index, ch) in format.char_indices() {
            let code = ch as u32;
            if code == 0 {
                break;
            }
            if code > 255 {
                return Err(bad_ui("game format strings must contain byte-valued code units"));
            }
            let _ = index;
            out.push(code as u8);
        }
        out
    };
    let mut output = String::new();
    let mut cursor = 0usize;
    let mut argument = 0usize;
    macro_rules! append_byte {
        ($byte:expr) => {{
            check_reserve(output.len(), 1)?;
            output.push(($byte & 255) as u8 as char);
        }};
    }
    while cursor < bytes.len() {
        let literal = bytes[cursor];
        if literal != b'%' {
            append_byte!(literal as i32);
            cursor += 1;
            continue;
        }
        cursor += 1;
        let mut flags = 0u32;
        let mut width = 0i32;
        let mut precision = -1i32;
        let specifier: u8 = loop {
            if cursor >= bytes.len() {
                return Err(bad_ui("unterminated game format specifier"));
            }
            let byte = bytes[cursor];
            cursor += 1;
            if byte == b'-' {
                flags |= 0x04;
                continue;
            }
            if byte == b'.' {
                let mut parsed = 0i32;
                while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                    parsed = parsed.wrapping_mul(10) + (bytes[cursor] - b'0') as i32;
                    cursor += 1;
                }
                precision = if parsed < 0 { -1 } else { parsed };
                continue;
            }
            if byte == b'0' {
                flags |= 0x80;
                continue;
            }
            if (b'1'..=b'9').contains(&byte) {
                let mut parsed = 0i32;
                let mut digit = byte;
                while digit.is_ascii_digit() {
                    parsed = parsed.wrapping_mul(10) + (digit - b'0') as i32;
                    if cursor >= bytes.len() {
                        return Err(bad_ui("unterminated game format specifier"));
                    }
                    digit = bytes[cursor];
                    cursor += 1;
                }
                width = parsed;
                cursor -= 1;
                continue;
            }
            break byte;
        };
        if specifier == b'%' {
            append_byte!(37);
            continue;
        }
        let value = args
            .get(argument)
            .ok_or_else(|| bad_ui(format!("missing argument {argument} for %{}", specifier as char)))?;
        match specifier {
            b'd' | b'i' => {
                let number = match value {
                    GameFormatArg::Int(number) => *number,
                    _ => {
                        return Err(bad_ui(format!(
                            "argument {argument} for %{} must be a number",
                            specifier as char
                        )))
                    }
                };
                add_int(&mut output, number, width, flags)?;
            }
            b'f' => {
                let number = match value {
                    GameFormatArg::Float(number) => *number,
                    _ => return Err(bad_ui(format!("argument {argument} for %f must be a number"))),
                };
                if !number.is_finite() || number.abs() > 2_147_483_647.0 {
                    return Err(bad_ui(format!(
                        "argument {argument} for %f is outside the source's safe int-cast range"
                    )));
                }
                add_float(&mut output, number, width, precision)?;
            }
            b's' => {
                let text = match value {
                    GameFormatArg::Text(text) => text.clone(),
                    _ => return Err(bad_ui(format!("argument {argument} for %s must be a string or null"))),
                };
                add_string(&mut output, text.as_deref(), width, precision)?;
            }
            _ => {
                let number = match value {
                    GameFormatArg::Int(number) => *number,
                    _ => {
                        return Err(bad_ui(format!(
                            "argument {argument} for %{} must be a number",
                            specifier as char
                        )))
                    }
                };
                append_byte!(number);
            }
        }
        argument += 1;
    }
    let nul = output.find('\0');
    let visible = match nul {
        Some(end) => &output[..end],
        None => output.as_str(),
    };
    Ok(visible.chars().take(BIG_BUFFER_BYTES - 1).collect())
}

/// Check the `Com_sprintf` destination bound.
fn check_reserve(current: usize, extra: usize) -> Result<(), ClientError> {
    if current + extra >= BIG_BUFFER_BYTES {
        return Err(bad_ui("game format exceeds the 32000-byte Com_sprintf buffer"));
    }
    Ok(())
}

/// Append a reversed-digit integer (`AddInt`).
fn add_int(output: &mut String, value: i32, width: i32, flags: u32) -> Result<(), ClientError> {
    let mut remaining = if value < 0 {
        value.wrapping_neg() as u32
    } else {
        value as u32
    };
    let mut reversed = Vec::new();
    loop {
        reversed.push(b'0' + (remaining % 10) as u8);
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    if value < 0 {
        reversed.push(b'-');
    }
    if flags & 0x04 == 0 {
        let padding = if width as usize > reversed.len() {
            width as usize - reversed.len()
        } else {
            0
        };
        check_reserve(output.len(), padding)?;
        let pad = if flags & 0x80 != 0 { '0' } else { ' ' };
        output.extend(std::iter::repeat_n(pad, padding));
    }
    for byte in reversed.iter().rev() {
        check_reserve(output.len(), 1)?;
        output.push(*byte as char);
    }
    if flags & 0x04 != 0 {
        let remaining_width = width.wrapping_sub(reversed.len() as i32);
        if remaining_width < 0 {
            return Err(bad_ui(
                "left-adjusted integer width would enter the source's negative padding loop",
            ));
        }
        check_reserve(output.len(), remaining_width as usize)?;
        let pad = if flags & 0x80 != 0 { '0' } else { ' ' };
        output.extend(std::iter::repeat_n(pad, remaining_width as usize));
    }
    Ok(())
}

/// Append a float with `bg_lib` digit extraction (`AddFloat`).
fn add_float(output: &mut String, value: f32, width: i32, precision: i32) -> Result<(), ClientError> {
    let mut remaining = if value < 0.0 { -value } else { value };
    #[allow(clippy::cast_possible_truncation)]
    let integer = remaining.trunc() as i32;
    let mut digits = if integer == 0 {
        vec![b'0']
    } else {
        let mut out = Vec::new();
        let mut rest = integer as u32;
        while rest != 0 {
            out.push(b'0' + (rest % 10) as u8);
            rest /= 10;
        }
        out.into_iter().rev().collect::<Vec<_>>()
    };
    if value < 0.0 {
        digits.insert(0, b'-');
    }
    let padding = if width as usize > digits.len() {
        width as usize - digits.len()
    } else {
        0
    };
    check_reserve(output.len(), padding)?;
    output.extend(std::iter::repeat_n(' ', padding));
    for byte in &digits {
        check_reserve(output.len(), 1)?;
        output.push(*byte as char);
    }
    let count = if precision < 0 { 6 } else { precision };
    if count > FLOAT_PRECISION_MAX {
        return Err(bad_ui("float precision would overflow AddFloat's 32-byte digit buffer"));
    }
    if count == 0 {
        return Ok(());
    }
    check_reserve(output.len(), 1)?;
    output.push('.');
    for _ in 0..count {
        remaining = f(remaining - remaining.trunc());
        remaining = f(remaining * 10.0);
        #[allow(clippy::cast_possible_truncation)]
        let digit = remaining.trunc() as i32 % 10;
        check_reserve(output.len(), 1)?;
        output.push((b'0' + digit.rem_euclid(10) as u8) as char);
    }
    Ok(())
}

/// Append a padded string (`AddString`).
fn add_string(output: &mut String, value: Option<&str>, width: i32, precision: i32) -> Result<(), ClientError> {
    let text = value.unwrap_or("(null)");
    let limit = if value.is_none() || precision < 0 {
        text.len()
    } else {
        precision as usize
    };
    let mut length = 0usize;
    for (index, ch) in text.char_indices() {
        if index >= limit {
            break;
        }
        let code = ch as u32;
        if code == 0 {
            break;
        }
        if code > 255 {
            return Err(bad_ui("game format strings must contain byte-valued code units"));
        }
        length = index + ch.len_utf8();
    }
    let visible = &text[..length.min(text.len())];
    check_reserve(output.len(), visible.len())?;
    output.push_str(visible);
    let padding = width.wrapping_sub(visible.len() as i32);
    if padding > 0 {
        check_reserve(output.len(), padding as usize)?;
        output.extend(std::iter::repeat_n(' ', padding as usize));
    }
    Ok(())
}

/// Byte cursor over a game-number scan (`NumberInput`).
struct NumberInput<'a> {
    /// Backing bytes (already validated as byte text by callers).
    bytes: &'a [u8],
    /// Current offset.
    offset: usize,
}

impl<'a> NumberInput<'a> {
    /// Build a cursor, rejecting non-byte text like the donor.
    fn new(text: &'a str) -> Result<Self, ClientError> {
        if text.chars().any(|c| c as u32 > 255) {
            return Err(bad_ui("Game numbers require byte characters"));
        }
        Ok(Self {
            bytes: text.as_bytes(),
            offset: 0,
        })
    }

    /// Peek the signed byte at the cursor (NUL past the end).
    fn byte(&self) -> i32 {
        if self.offset >= self.bytes.len() {
            return 0;
        }
        let byte = self.bytes[self.offset];
        if byte < 128 {
            i32::from(byte)
        } else {
            i32::from(byte) - 256
        }
    }

    /// Consume one signed byte.
    fn take(&mut self) -> i32 {
        let byte = self.byte();
        self.offset += 1;
        byte
    }

    /// Skip bytes at or below space (excluding NUL).
    fn skip_whitespace(&mut self) {
        while self.byte() <= 32 && self.byte() != 0 {
            self.offset += 1;
        }
    }

    /// Consume an optional sign, returning its multiplier.
    fn sign(&mut self) -> i32 {
        let byte = self.byte();
        if byte != 43 && byte != 45 {
            return 1;
        }
        self.offset += 1;
        if byte == 45 {
            -1
        } else {
            1
        }
    }
}

/// Scan a decimal float prefix with binary32 steps (`readFloat`, scalar mode).
fn game_atof(text: &str) -> Result<f32, ClientError> {
    let mut input = NumberInput::new(text)?;
    input.skip_whitespace();
    if input.byte() == 0 {
        return Ok(0.0);
    }
    let sign = input.sign();
    let mut value = 0.0f32;
    let mut character = input.byte();
    if input.byte() != 46 {
        loop {
            character = input.take();
            if !(48..=57).contains(&character) {
                break;
            }
            value = f(f(value * 10.0) + (character - 48) as f32);
        }
    } else {
        input.offset += 1;
    }
    if character == 46 {
        let mut fraction = f(0.1);
        loop {
            character = input.take();
            if !(48..=57).contains(&character) {
                break;
            }
            value = f(value + f((character - 48) as f32 * fraction));
            fraction = f(fraction * f(0.1));
        }
    }
    Ok(f(value * sign as f32))
}

/// Scan a decimal integer prefix with wrapping arithmetic (`gameAtoi`).
fn game_atoi(text: &str) -> Result<i32, ClientError> {
    let mut input = NumberInput::new(text)?;
    input.skip_whitespace();
    if input.byte() == 0 {
        return Ok(0);
    }
    let sign = input.sign();
    let mut value = 0i32;
    loop {
        let character = input.take();
        if !(48..=57).contains(&character) {
            break;
        }
        value = value.wrapping_mul(10) + character - 48;
    }
    Ok(value.wrapping_mul(sign))
}

impl UiScript {
    /// Build a script from raw text with no eager tokens.
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        Self {
            text: text.to_string(),
            tokens: Vec::new(),
            truncated: false,
        }
    }
}

impl UiGlobalAssets {
    /// Empty assets (moved-from placeholder and test seed).
    #[must_use]
    pub fn empty() -> Self {
        Self {
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

impl UiMenuRegistrationEvent {
    /// Referenced asset path.
    #[must_use]
    pub fn path(&self) -> Option<&str> {
        match self {
            UiMenuRegistrationEvent::Font { reference, .. } => reference.path.as_deref(),
            UiMenuRegistrationEvent::Picture { reference, .. } => reference.path.as_deref(),
            UiMenuRegistrationEvent::Sound { reference, .. } => reference.path.as_deref(),
            UiMenuRegistrationEvent::Model { reference, .. } => reference.path.as_deref(),
        }
    }
}

/// A script diagnostic (`ScriptDiagnostic`).
///
/// Runtime-visible script diagnostic (sibling `ScriptDiagnostic` equivalent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiScriptDiagnostic {
    /// Whether this is a warning (`true`) or an error (`false`).
    pub warning: bool,
    /// Diagnostic message.
    pub message: String,
    /// Diagnostic location.
    pub location: SourceLocation,
}

impl PartialEq for UiMenuMemoryOwnership {
    /// Kind equality; QVM32 owners compare by identity like the donor.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (UiMenuMemoryOwnership::Unaccounted, UiMenuMemoryOwnership::Unaccounted) => true,
            (UiMenuMemoryOwnership::Qvm32 { memory: left }, UiMenuMemoryOwnership::Qvm32 { memory: right }) => {
                left.same_memory(right)
            }
            _ => false,
        }
    }
}

impl Eq for UiMenuMemoryOwnership {}

impl UiMenuDefinitions {
    /// Empty definitions (moved-from placeholder and test seed).
    #[must_use]
    pub fn empty() -> Self {
        Self {
            memory: UiMenuMemoryOwnership::Unaccounted,
            menus: Vec::new(),
            assets: UiGlobalAssets::empty(),
            loaded_files: Vec::new(),
            diagnostics: Vec::new(),
            registration: UiMenuRegistrationState::Completed { events: Vec::new() },
            font_registered: false,
        }
    }
}

/// A key or character event (`UiKeyEvent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiKeyEvent {
    /// Physical key press or release.
    Key {
        /// Key code (0..=0x7fff).
        code: i32,
        /// Press (`true`) or release (`false`).
        down: bool,
    },
    /// Character input (single byte).
    Character {
        /// Character code (0..=255).
        code: i32,
    },
}

/// UI-local sampled-sound handle (`PcmSound`).
///
/// The donor threads the audio subsystem's `PcmSound` through opaquely; the
/// runtime never reads its fields, so this UI-local struct carries only the
/// donor-visible registration identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PcmSound {
    /// Registered path, `None` for null.
    pub path: Option<String>,
    /// Registration sequence for test observability.
    pub handle: u32,
}

/// UI-local scene-model handle (`SceneModel`).
///
/// Like [`PcmSound`], the donor-visible surface is registration identity;
/// presentation fields stay with the content subsystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneModel {
    /// Registered path, `None` for null.
    pub path: Option<String>,
    /// Registration sequence for test observability.
    pub handle: u32,
}

/// Diagnostic fallback model (`DEFAULT_MODEL`).
#[must_use]
pub fn default_model() -> SceneModel {
    SceneModel { path: None, handle: 0 }
}

/// A prepared cinematic asset (`UiCinematicAsset`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiCinematicAsset {
    /// Asset path.
    pub path: String,
}

/// A playing cinematic instance (`UiCinematicInstance`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiCinematicInstance {
    /// Source asset.
    pub asset: UiCinematicAsset,
    /// Engine instance index (donor `handle.index`).
    pub handle: i32,
}

/// Local sound selection (`PcmSound | number | undefined`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiLocalSound {
    /// Sampled sound.
    Pcm(PcmSound),
    /// Numeric source handle.
    Handle(i32),
}

/// UI audio host (`UiRuntimeAudio`).
///
/// `Promise -> sync`: `startBackground` resolves synchronously.
pub trait UiRuntimeAudio {
    /// Play a local sound (`None` plays the configured fallback).
    fn play_local(&mut self, sound: Option<UiLocalSound>);
    /// Start background music (`None` stops).
    fn start_background(&mut self, path: Option<&str>);
    /// Stop background music.
    fn stop_background(&mut self);
}

/// Error text for numeric handle use without a source owner.
///
/// Host implementors return this from the numeric handle methods when
/// [`UiHandleKind::Diagnostic`] is active.
pub const NUMERIC_HANDLES: &str = "Numeric UI resource use requires the actual source handle owners";

/// Resource-handle ownership (`UiRuntimeResources["handles"].kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiHandleKind {
    /// Handles are unavailable; numeric use is an error.
    Diagnostic,
    /// The source owns numeric handles.
    Source,
}

/// UI resource host (`UiRuntimeResources`).
///
/// `Promise -> sync`: every `register*`/`prepareCinematic` call resolves
/// synchronously; `*`ed getters were already synchronous.
pub trait UiRuntimeResources {
    /// Handle ownership kind.
    fn handle_kind(&self) -> UiHandleKind;
    /// Resolve a picture to its numeric handle (`pictureHandle`).
    ///
    /// Fails when [`UiHandleKind::Diagnostic`], like the donor's
    /// `sourceHandles()` guard.
    fn picture_handle(&mut self, picture: Option<PictureAsset>) -> Result<i32, ClientError>;
    /// Look up a picture by handle (`pictureForHandle`).
    ///
    /// Fails when [`UiHandleKind::Diagnostic`].
    fn picture_for_handle(&self, handle: i32) -> Result<Option<PictureAsset>, ClientError>;
    /// Look up a model by handle (`modelForHandle`).
    ///
    /// Fails when [`UiHandleKind::Diagnostic`].
    fn model_for_handle(&self, handle: i32) -> Result<SceneModel, ClientError>;
    /// Register a font.
    fn register_font(&mut self, path: Option<&str>, point_size: i32);
    /// Register a picture.
    fn register_picture(&mut self, path: Option<&str>) -> Option<PictureAsset>;
    /// Look up an already-registered picture.
    fn registered_picture(&self, path: Option<&str>) -> Option<PictureAsset>;
    /// Register a sound.
    fn register_sound(&mut self, path: Option<&str>) -> Option<PcmSound>;
    /// Look up an already-registered sound.
    fn registered_sound(&self, path: Option<&str>) -> Option<PcmSound>;
    /// Register a model.
    fn register_model(&mut self, path: Option<&str>) -> SceneModel;
    /// Look up an already-registered model.
    fn registered_model(&self, path: Option<&str>) -> Option<SceneModel>;
    /// Prepare a cinematic asset.
    fn prepare_cinematic(&mut self, path: &str) -> UiCinematicAsset;
}

/// UI cinematic host (`UiRuntimeCinematics`, already synchronous).
pub trait UiRuntimeCinematics {
    /// Start playing an asset in a rectangle.
    fn play(&mut self, asset: &UiCinematicAsset, rect: &UiRect) -> Option<UiCinematicInstance>;
    /// Advance an instance to `time`.
    fn run(&mut self, handle: i32, time: i32);
    /// Paint an instance.
    fn draw(&mut self, handle: i32, rect: &UiRect, draw: &mut Draw2D);
    /// Stop an instance.
    fn stop(&mut self, handle: i32);
}

/// Model paint callback (`(request: UiModelPaintRequest) => void`).
#[allow(clippy::type_complexity)]
pub type UiModelPainter = Box<dyn for<'a, 'd, 's> FnMut(UiModelPaintRequest<'a, 'd, 's>)>;

/// Built-in widget pictures (`UiWidgetAssets`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiWidgetAssets {
    /// White shader.
    pub white_shader: PictureAsset,
    /// Gradient bar.
    pub gradient_bar: PictureAsset,
    /// Scrollbar track.
    pub scroll_bar: PictureAsset,
    /// Scrollbar down arrow.
    pub scroll_bar_arrow_down: PictureAsset,
    /// Scrollbar up arrow.
    pub scroll_bar_arrow_up: PictureAsset,
    /// Scrollbar left arrow.
    pub scroll_bar_arrow_left: PictureAsset,
    /// Scrollbar right arrow.
    pub scroll_bar_arrow_right: PictureAsset,
    /// Scrollbar thumb.
    pub scroll_bar_thumb: PictureAsset,
    /// Slider bar.
    pub slider_bar: PictureAsset,
    /// Slider thumb.
    pub slider_thumb: PictureAsset,
}

/// Model paint request (`UiModelPaintRequest`).
pub struct UiModelPaintRequest<'a, 'd, 's> {
    /// Target draw context.
    pub draw: &'d mut Draw2D<'s>,
    /// Model to paint.
    pub model: &'a SceneModel,
    /// Destination rectangle.
    pub rect: UiRect,
    /// Display time.
    pub time: i32,
    /// Yaw angle.
    pub angle: i32,
    /// Horizontal FOV (0 selects the viewport after 640-adjust).
    pub field_of_view_x: f32,
    /// Vertical FOV (0 selects the viewport after 640-adjust).
    pub field_of_view_y: f32,
}

/// Owner-draw paint request (`UiOwnerDrawPaintRequest`).
///
/// `Promise -> sync`, and the donor's lazy `background` closure is resolved
/// eagerly (see module docs).
pub struct UiOwnerDrawPaintRequest<'d, 's> {
    /// Target draw context.
    pub draw: &'d mut Draw2D<'s>,
    /// Destination rectangle.
    pub rect: UiRect,
    /// Text x.
    pub text_x: f32,
    /// Text y.
    pub text_y: f32,
    /// Owner-draw id.
    pub owner_draw: i32,
    /// Owner-draw flags.
    pub owner_draw_flags: i32,
    /// Image alignment.
    pub alignment: i32,
    /// Item scratch value.
    pub special: f32,
    /// Text scale.
    pub text_scale: f32,
    /// Text color.
    pub color: Vec4,
    /// Resolved background picture.
    pub background: Option<PictureAsset>,
    /// Text style.
    pub text_style: i32,
}

/// Key-binding host (`UiRuntimeBindings`, already synchronous).
pub trait UiRuntimeBindings {
    /// Display name for a key.
    fn key_name(&mut self, key: i32) -> String;
    /// Command bound to a key.
    fn get_binding(&mut self, key: i32) -> String;
    /// Bind a command to a key.
    fn set_binding(&mut self, key: i32, command: &str);
    /// Overstrike state.
    fn get_overstrike(&mut self) -> bool;
    /// Set overstrike state.
    fn set_overstrike(&mut self, enabled: bool);
}

/// One feeder cell (`UiRuntimeFeederItem`).
///
/// `Promise -> sync` on the producing methods.
#[derive(Debug, Clone, PartialEq)]
pub struct UiRuntimeFeederItem {
    /// Cell text (`None` for null).
    pub text: Option<String>,
    /// Cell picture.
    pub picture: Option<PictureAsset>,
}

/// List-box feeder host (`UiRuntimeFeeder`).
///
/// `Promise -> sync`: `item`, `image`, and `select` resolve synchronously.
pub trait UiRuntimeFeeder {
    /// Row count for a feeder.
    fn count(&mut self, feeder: f32) -> i32;
    /// Cell at (feeder, index, column).
    fn item(&mut self, feeder: f32, index: i32, column: i32) -> Option<UiRuntimeFeederItem>;
    /// Image row picture.
    fn image(&mut self, feeder: f32, index: i32) -> Option<PictureAsset>;
    /// Record a selection.
    fn select(&mut self, feeder: f32, index: i32);
}

/// Owner-draw key result (`UiOwnerDrawKeyResult`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiOwnerDrawKeyResult {
    /// Whether the key was handled.
    pub handled: bool,
    /// Updated item scratch value.
    pub special: f32,
}

/// Owner-draw host (`UiRuntimeOwnerDraw`).
///
/// `Promise -> sync`: `handleKey` and `paint` resolve synchronously.
pub trait UiRuntimeOwnerDraw {
    /// Whether flags select a visible draw.
    fn visible(&mut self, flags: i32) -> bool;
    /// Owner-draw text width contribution.
    fn width(&mut self, owner_draw: i32, scale: f32) -> i32;
    /// Owner-draw value for color ranges.
    fn value(&mut self, owner_draw: i32) -> f32;
    /// Handle a key.
    fn handle_key(&mut self, owner_draw: i32, flags: i32, special: f32, key: i32) -> UiOwnerDrawKeyResult;
    /// Paint.
    fn paint(&mut self, request: UiOwnerDrawPaintRequest<'_, '_>);
    /// Close an owner-draw cinematic.
    fn close_cinematic(&mut self, owner_draw: i32);
}

/// External-script context (`UiExternalScriptContext`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiExternalScriptContext {
    /// Running menu name.
    pub menu_name: Option<String>,
    /// Running item name.
    pub item_name: Option<String>,
}

/// Script-token cursor (`UiScriptCursor`).
pub trait UiScriptCursor {
    /// Consumed token count (`position`).
    fn position(&self) -> usize;
    /// Tokens remaining (lookahead; does not consume).
    fn remaining(&self) -> usize;
    /// Peek the next token (lookahead; does not consume).
    fn peek(&self) -> Option<UiScriptToken>;
    /// Consume and return the next token.
    fn next_token(&mut self) -> Option<UiScriptToken>;
    /// `String_Parse`: `None` means no token; `Some(None)` means allocation
    /// returned NULL; `Some(Some(_))` is the parsed string.
    fn string(&mut self) -> Option<Option<String>>;
}

/// External-script host (`UiExternalScriptHost`).
///
/// `Promise -> sync`: `run` resolves synchronously. Mirrors
/// `displayContextDef_t.runScript(char **p)` after the shared marker token
/// was consumed.
pub trait UiExternalScriptHost {
    /// Run game-specific script tokens.
    fn run(&mut self, cursor: &mut dyn UiScriptCursor, context: &UiExternalScriptContext);
}

/// A cvar value snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct UiCvarValue {
    /// String value.
    pub value: String,
    /// Numeric value.
    pub numeric_value: f32,
}

/// Cvar host (`Pick<CvarRegistry, "get" | "set" | "reset">`, synchronous).
pub trait UiCvarRegistry {
    /// Look up a cvar.
    fn get(&self, name: &str) -> Option<UiCvarValue>;
    /// Set a cvar.
    fn set(&mut self, name: &str, value: &str, force: bool);
    /// Reset a cvar.
    fn reset(&mut self, name: &str, force: bool);
}

/// Command origin (`CommandOrigin`, local subset).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiCommandOrigin {
    /// Local console.
    LocalConsole,
    /// Server console.
    ServerConsole,
    /// Local seat.
    LocalSeat {
        /// Seat.
        seat: SeatId,
        /// Client.
        client: ClientId,
    },
    /// Remote client.
    RemoteClient {
        /// Client.
        client: ClientId,
    },
    /// Script execution.
    Script {
        /// Script name.
        name: String,
        /// Caller origin.
        caller: Box<UiCommandOrigin>,
    },
}

/// Buffered-command context (`CommandContext`).
///
/// The donor's optional `producer` (carrying an unportable `symbol`
/// instance) is omitted; command text and origin are preserved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandContext {
    /// Session.
    pub session: SessionId,
    /// Origin.
    pub origin: UiCommandOrigin,
}

/// Command-buffer host (`Pick<CommandBuffer, "append">`, synchronous).
pub trait UiCommandBuffer {
    /// Append text with its execution context.
    fn append(&mut self, text: &str, context: &CommandContext);
}

/// Source-parser state (`CommonParseState`, runtime-observable subset).
///
/// The runtime only observes the shared line counter through script-token
/// locations; tokenization itself is implemented by
/// [`RuntimeScriptCursor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiSourceParser {
    /// Current line.
    pub line: i32,
}

/// Runtime context (`UiRuntimeContext`).
///
/// `Promise -> sync`: the UI `pause` callback resolves synchronously.
pub enum UiRuntimeContext {
    /// Full UI context with bindings and pause control.
    Ui {
        /// Binding host.
        bindings: Box<dyn UiRuntimeBindings>,
        /// Pause control.
        pause: Box<dyn FnMut(bool)>,
    },
    /// Cgame context (bindings/exec unavailable).
    Cgame,
}

/// Diagnostic print hook (`print?: (text: string) => void`).
pub type UiPrintHook = Option<Box<dyn FnMut(&str)>>;

/// Cvar numeric-value override (`cvarValue?: (name: string) => number`).
pub type UiCvarValueHook = Option<Box<dyn FnMut(&str) -> f32>>;

/// Runtime construction options (`UiRuntimeOptions`).
///
/// `Promise -> sync`: all `async` host hooks (`resources`, `audio`,
/// `cinematics` setup, `paintModel`, `feeder`, `ownerDraw`,
/// `externalScript`, `pause`) are synchronous here.
pub struct UiRuntimeOptions {
    /// Initial menu definitions (moved into the runtime on creation).
    pub definitions: UiMenuDefinitions,
    /// Shared source-parser state.
    pub source_parser: Option<UiSourceParser>,
    /// Diagnostic print hook.
    pub print: UiPrintHook,
    /// Cvar numeric-value override.
    pub cvar_value: UiCvarValueHook,
    /// Cvar host.
    pub cvars: Box<dyn UiCvarRegistry>,
    /// Command-buffer host.
    pub commands: Box<dyn UiCommandBuffer>,
    /// Command execution context.
    pub command_context: CommandContext,
    /// Resource host.
    pub resources: Box<dyn UiRuntimeResources>,
    /// Font set.
    pub fonts: FontSet,
    /// Widget pictures.
    pub widget_assets: UiWidgetAssets,
    /// Fallback picture.
    pub zero_picture: PictureAsset,
    /// Audio host.
    pub audio: Box<dyn UiRuntimeAudio>,
    /// Cinematic host.
    pub cinematics: Box<dyn UiRuntimeCinematics>,
    /// Model painter.
    pub paint_model: UiModelPainter,
    /// Runtime context.
    pub context: UiRuntimeContext,
    /// Feeder host.
    pub feeder: Box<dyn UiRuntimeFeeder>,
    /// Owner-draw host.
    pub owner_draw: Box<dyn UiRuntimeOwnerDraw>,
    /// External-script host.
    pub external_script: Box<dyn UiExternalScriptHost>,
    /// Team-color provider.
    pub get_team_color: Box<dyn FnMut() -> Vec4>,
}

/// A captured (pinned) menu handle (`UiCapturedMenu`).
///
/// The donor keys handles by object identity; owned Rust definitions pin the
/// menu slot instead (slots survive repopulation like the donor's static
/// array).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UiCapturedMenu {
    /// Pinned menu slot.
    index: usize,
}

/// Item behavior snapshot (`UiRuntimeItemBehaviorSnapshot`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiRuntimeItemBehaviorSnapshot {
    /// List-box positions.
    ListBox {
        /// First visible row.
        start_position: i32,
        /// Last painted row.
        end_position: i32,
        /// Highlighted row.
        cursor_position: i32,
        /// Trailing paint padding.
        draw_padding: i32,
    },
    /// Edit paint offset.
    Edit {
        /// Paint scroll offset.
        paint_offset: i32,
    },
    /// Other behaviors.
    Other,
}

/// Item snapshot (`UiRuntimeItemSnapshot`).
#[derive(Debug, Clone, PartialEq)]
pub struct UiRuntimeItemSnapshot {
    /// Item name.
    pub name: Option<String>,
    /// Item group.
    pub group: Option<String>,
    /// Window flags.
    pub flags: i32,
    /// Screen rectangle.
    pub rect: UiRect,
    /// Client rectangle.
    pub client_rect: UiRect,
    /// Foreground color.
    pub fore_color: Vec4,
    /// Background color.
    pub back_color: Vec4,
    /// Border color.
    pub border_color: Vec4,
    /// Resolved background.
    pub background: Option<PictureAsset>,
    /// Edit cursor / list selection.
    pub cursor_position: i32,
    /// Item scratch value.
    pub special: f32,
    /// Whether the enable cvar test passes.
    pub enabled: bool,
    /// Whether the show cvar test passes.
    pub shown: bool,
    /// Behavior state.
    pub behavior: UiRuntimeItemBehaviorSnapshot,
}

/// Menu snapshot (`UiRuntimeMenuSnapshot`).
#[derive(Debug, Clone, PartialEq)]
pub struct UiRuntimeMenuSnapshot {
    /// Menu name.
    pub name: Option<String>,
    /// Window flags.
    pub flags: i32,
    /// Screen rectangle.
    pub rect: UiRect,
    /// Focused item index.
    pub cursor_item: i32,
    /// Item snapshots.
    pub items: Vec<UiRuntimeItemSnapshot>,
}

/// Runtime snapshot (`UiRuntimeSnapshot`).
#[derive(Debug, Clone, PartialEq)]
pub struct UiRuntimeSnapshot {
    /// Focused menu name.
    pub focused_menu: Option<String>,
    /// Open-stack menu names.
    pub open_stack: Vec<Option<String>>,
    /// Menu snapshots.
    pub menus: Vec<UiRuntimeMenuSnapshot>,
}

/// A paint frame (`UiRuntimeFrame`).
pub struct UiRuntimeFrame<'a> {
    /// Display time in milliseconds.
    pub time: i32,
    /// Frame time in milliseconds.
    pub frame_time: i32,
    /// Target draw context.
    pub draw: Draw2D<'a>,
}

/// Pointer cursor shape (`"arrow" | "sizer"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiCursorType {
    /// Normal arrow.
    Arrow,
    /// Resize sizer.
    Sizer,
}

/// Definition-reset scope (`"menus" | "strings"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiDefinitionReset {
    /// Drop menus only.
    Menus,
    /// Drop menus and string state.
    Strings,
}

/// Menu activation selector (`string | null | (() => string)`).
pub enum UiMenuSelector {
    /// Activate by name.
    Name(String),
    /// Null selector (clears focus, matches nothing, like the donor).
    Null,
    /// Resolve the name through a callback (donor `() => string`).
    Resolve(Box<dyn FnMut() -> String>),
}

/// An owned item handle (`ItemState`).
///
/// The donor's `ItemState` aliases a live definition plus a proxied window;
/// owned Rust definitions address the item by (menu, item) indices instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ItemState {
    /// Menu slot.
    pub menu_index: usize,
    /// Item index within the menu.
    pub item_index: usize,
}

/// Script owner (menu plus optional item).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScriptOwner {
    /// Owner menu slot.
    menu: usize,
    /// Owner item, when running item script.
    item: Option<ItemState>,
}

/// Key-binding rows (`BindingState`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct BindingState {
    /// Bound command.
    command: String,
    /// Primary key.
    first: i32,
    /// Secondary key.
    second: i32,
}

/// Pointer-capture state (`CaptureState`).
#[derive(Debug, Clone, Copy, PartialEq)]
enum CaptureState {
    /// No capture.
    Idle,
    /// List auto-scroll capture.
    ListAuto {
        /// Captured item.
        item: ItemState,
        /// Capture key.
        key: i32,
        /// Capture start x.
        x_start: f32,
        /// Capture start y.
        y_start: f32,
    },
    /// List thumb-drag capture.
    ListThumb {
        /// Captured item.
        item: ItemState,
        /// Capture key.
        key: i32,
        /// Capture start x.
        x_start: f32,
        /// Capture start y.
        y_start: f32,
    },
    /// Slider thumb-drag capture.
    SliderThumb {
        /// Captured item.
        item: ItemState,
        /// Capture key.
        key: i32,
        /// Capture start x.
        x_start: f32,
        /// Capture start y.
        y_start: f32,
    },
}

impl CaptureState {
    /// Captured item, if any.
    fn item(self) -> Option<ItemState> {
        match self {
            CaptureState::Idle => None,
            CaptureState::ListAuto { item, .. }
            | CaptureState::ListThumb { item, .. }
            | CaptureState::SliderThumb { item, .. } => Some(item),
        }
    }
}

/// `COM_Parse` script cursor (`RuntimeScriptCursor`).
///
/// Tokenizes with the donor's `CommonParseState.parse(cursor, false)`
/// semantics (no line breaks, `//` and `/* */` comments, quoted strings,
/// 1024-byte storage rules) over the 8-bit script prefix.
#[derive(Debug, Clone)]
pub struct RuntimeScriptCursor {
    /// Script bytes (8-bit prefix, truncated to [`MAX_SCRIPT_BYTES`]).
    source: Vec<u8>,
    /// Byte offset, `None` at exhaustion.
    offset: Option<usize>,
    /// Consumed token count.
    index: usize,
    /// Shared line counter value.
    line: i32,
    /// String memory owner.
    memory: UiMenuMemoryOwnership,
}

impl RuntimeScriptCursor {
    /// Build a cursor over script text with a starting line and memory.
    pub fn new(text: &str, line: i32, memory: &UiMenuMemoryOwnership) -> Result<Self, ClientError> {
        let valid = quake_string(text, None)?;
        let truncated: String = valid.chars().take(MAX_SCRIPT_BYTES).collect();
        Ok(Self {
            source: truncated.bytes().collect(),
            offset: Some(0),
            index: 0,
            line,
            memory: memory.clone(),
        })
    }

    /// Current line counter (write back to the shared parser when done).
    #[must_use]
    pub fn line(&self) -> i32 {
        self.line
    }

    /// Signed source byte (Linux/QVM profile; NUL past the end).
    fn signed_byte(source: &[u8], offset: usize) -> i32 {
        if offset >= source.len() {
            return 0;
        }
        let byte = source[offset];
        if byte < 128 {
            i32::from(byte)
        } else {
            i32::from(byte) - 256
        }
    }

    /// Parse one token from an offset, updating the line counter.
    fn parse_at(source: &[u8], offset: Option<usize>, line: &mut i32) -> Result<(Option<usize>, String), ClientError> {
        let Some(mut data) = offset else {
            return Ok((None, String::new()));
        };
        let mut has_new_lines = false;
        let mut token = String::new();
        loop {
            let mut c: i32;
            loop {
                c = Self::signed_byte(source, data);
                if c > 32 {
                    break;
                }
                if c == 0 {
                    return Ok((None, String::new()));
                }
                if c == 10 {
                    *line = line.wrapping_add(1);
                    has_new_lines = true;
                }
                data += 1;
            }
            if has_new_lines {
                return Ok((Some(data), String::new()));
            }
            if c == 47 && Self::signed_byte(source, data + 1) == 47 {
                data += 2;
                while {
                    c = Self::signed_byte(source, data);
                    c != 0 && c != 10
                } {
                    data += 1;
                }
            } else if c == 47 && Self::signed_byte(source, data + 1) == 42 {
                data += 2;
                while Self::signed_byte(source, data) != 0
                    && (Self::signed_byte(source, data) != 42 || Self::signed_byte(source, data + 1) != 47)
                {
                    data += 1;
                }
                if Self::signed_byte(source, data) != 0 {
                    data += 2;
                }
            } else {
                break;
            }
        }
        let c = Self::signed_byte(source, data);
        if c == 34 {
            data += 1;
            loop {
                let byte = Self::signed_byte(source, data);
                data += 1;
                if byte == 34 || byte == 0 {
                    if token.len() == MAX_TOKEN_CHARS {
                        return Err(bad_ui("COM_Parse quoted token terminator exceeds 1024-byte storage"));
                    }
                    let next = if byte == 0 { None } else { Some(data) };
                    return Ok((next, token));
                }
                if token.len() < MAX_TOKEN_CHARS {
                    token.push((byte & 255) as u8 as char);
                }
            }
        }
        loop {
            if token.len() < MAX_TOKEN_CHARS && data < source.len() {
                token.push(source[data] as char);
            }
            data += 1;
            let next = Self::signed_byte(source, data);
            if next == 10 {
                *line = line.wrapping_add(1);
            }
            if next <= 32 {
                break;
            }
        }
        if token.len() == MAX_TOKEN_CHARS {
            token.clear();
        }
        Ok((Some(data), token))
    }

    /// Parse the next raw string (`rawString`).
    pub fn raw_string(&mut self) -> Option<String> {
        let token = self.next_token()?;
        if token.text.is_empty() {
            return None;
        }
        quake_string(&token.text, Some(&token.location)).ok()
    }
}

impl UiScriptCursor for RuntimeScriptCursor {
    /// Consumed token count.
    fn position(&self) -> usize {
        self.index
    }

    /// Tokens remaining (fresh-parser lookahead).
    fn remaining(&self) -> usize {
        let mut offset = self.offset;
        let mut line = 0i32;
        let mut count = 0usize;
        while let Ok((next, token)) = Self::parse_at(&self.source, offset, &mut line) {
            if token.is_empty() {
                break;
            }
            count += 1;
            offset = next;
        }
        count
    }

    /// Peek the next token (fresh-parser lookahead).
    fn peek(&self) -> Option<UiScriptToken> {
        let mut line = 0i32;
        let (_, token) = Self::parse_at(&self.source, self.offset, &mut line).ok()?;
        if token.is_empty() {
            return None;
        }
        Some(UiScriptToken {
            text: token,
            location: SourceLocation {
                path: "<UI script>".to_string(),
                line: self.line as usize,
                column: 1,
            },
        })
    }

    /// Consume and return the next token.
    fn next_token(&mut self) -> Option<UiScriptToken> {
        let (next, token) = Self::parse_at(&self.source, self.offset, &mut self.line).ok()?;
        self.offset = next;
        if token.is_empty() {
            return None;
        }
        self.index += 1;
        Some(UiScriptToken {
            text: token,
            location: SourceLocation {
                path: "<UI script>".to_string(),
                line: self.line as usize,
                column: 1,
            },
        })
    }

    /// `String_Parse` with memory allocation.
    fn string(&mut self) -> Option<Option<String>> {
        let text = self.raw_string()?;
        match &self.memory {
            UiMenuMemoryOwnership::Unaccounted => Some(Some(text)),
            UiMenuMemoryOwnership::Qvm32 { memory } => Some(memory.string_alloc(Some(text.as_str())).ok()?),
        }
    }
}

/// Bindable commands (`BIND_COMMANDS`).
const BIND_COMMANDS: &[&str] = &[
    "+scores",
    "+button2",
    "+speed",
    "+forward",
    "+back",
    "+moveleft",
    "+moveright",
    "+moveup",
    "+movedown",
    "+left",
    "+right",
    "+strafe",
    "+lookup",
    "+lookdown",
    "+mlook",
    "centerview",
    "+zoom",
    "weapon 1",
    "weapon 2",
    "weapon 3",
    "weapon 4",
    "weapon 5",
    "weapon 6",
    "weapon 7",
    "weapon 8",
    "weapon 9",
    "weapon 10",
    "weapon 11",
    "weapon 12",
    "weapon 13",
    "+attack",
    "weapprev",
    "weapnext",
    "+button3",
    "+button4",
    "prevTeamMember",
    "nextTeamMember",
    "nextOrder",
    "confirmOrder",
    "denyOrder",
    "taskOffense",
    "taskDefense",
    "taskPatrol",
    "taskCamp",
    "taskFollow",
    "taskRetrieve",
    "taskEscort",
    "taskOwnFlag",
    "taskSuicide",
    "tauntKillInsult",
    "tauntPraise",
    "tauntTaunt",
    "tauntDeathInsult",
    "tauntGauntlet",
    "scoresUp",
    "scoresDown",
    "messagemode",
    "messagemode2",
    "messagemode3",
    "messagemode4",
];

/// Load binding rows from the host (`loadBindings`).
fn load_bindings(host: &mut dyn UiRuntimeBindings) -> Result<Vec<BindingState>, ClientError> {
    let mut bindings: Vec<BindingState> = BIND_COMMANDS
        .iter()
        .map(|command| BindingState {
            command: command.to_string(),
            first: NO_BINDING,
            second: NO_BINDING,
        })
        .collect();
    for binding in &mut bindings {
        for key in 0..256 {
            let current = quake_string(&host.get_binding(key), None)?;
            let trimmed = current.chars().take(255).collect::<String>();
            if equal_name(Some(&trimmed), &binding.command) {
                if binding.first == NO_BINDING {
                    binding.first = key;
                } else {
                    binding.second = key;
                    break;
                }
            }
        }
    }
    Ok(bindings)
}

/// Empty binding rows for cgame contexts.
fn empty_bindings() -> Vec<BindingState> {
    BIND_COMMANDS
        .iter()
        .map(|command| BindingState {
            command: command.to_string(),
            first: NO_BINDING,
            second: NO_BINDING,
        })
        .collect()
}

/// Whether an item matches a show/fade/transition group name (`matchesItem`).
fn matches_item(item: &UiItemDefinition, name: &str) -> bool {
    equal_name(item.window().name().as_deref(), name) || equal_name(item.window().group().as_deref(), name)
}

/// Place an item at a parent-relative screen origin (`setItemScreenCoords`).
fn set_item_screen_coords(item: &UiItemDefinition, mut x: f32, mut y: f32) {
    if item.window().border() != 0 {
        x = f(x + item.window().border_size());
        y = f(y + item.window().border_size());
    }
    let window = item.window();
    let rect = window.rect();
    let client = window.client_rect();
    rect.set_x(f(x + client.x()));
    rect.set_y(f(y + client.y()));
    rect.set_width(client.width());
    rect.set_height(client.height());
    let text = item.text_rect();
    text.set_width(0.0);
    text.set_height(0.0);
}

/// Team Arena shared-menu runtime (`UiRuntime`).
///
/// Owns its menu definitions and resolved resources; hosts provide cvars,
/// commands, resources, fonts, audio, cinematics, feeders, owner-draw, and
/// external scripts. `Promise -> sync` on every donor `async` method; the
/// donor's mid-operation `opened()` re-checks are dropped because hosts never
/// receive the runtime, so disposal cannot interleave.
pub struct UiRuntime {
    /// Host options (definitions move to [`UiRuntime::definitions`]).
    options: UiRuntimeOptions,
    /// Owned definitions.
    definitions: UiMenuDefinitions,
    /// Active menu prefix length.
    active_menu_count: usize,
    /// Resolved pictures by resource key.
    pictures: HashMap<String, Option<PictureAsset>>,
    /// Resolved sounds by resource key.
    sounds: HashMap<String, Option<PcmSound>>,
    /// Resolved models by resource key.
    models: HashMap<String, SceneModel>,
    /// Prepared cinematics by path.
    cinematics: HashMap<String, UiCinematicAsset>,
    /// Open-menu stack (menu slots; only grows until a strings reset).
    open_stack: Vec<usize>,
    /// Binding rows.
    bindings: Vec<BindingState>,
    /// Display cursor x.
    display_cursor_x: f32,
    /// Display cursor y.
    display_cursor_y: f32,
    /// Display time in milliseconds.
    real_time: i32,
    /// Last list-box click time.
    last_list_box_click_time: i32,
    /// Item under edit.
    editing_item: Option<ItemState>,
    /// Item awaiting a binding key.
    binding_item: Option<ItemState>,
    /// Whether a binding key is awaited.
    waiting_for_key: bool,
    /// Pointer capture.
    capture: CaptureState,
    /// Next auto-scroll time.
    next_scroll_time: i32,
    /// Next auto-scroll acceleration time.
    next_scroll_adjust_time: i32,
    /// Auto-scroll interval.
    scroll_adjust_value: i32,
    /// Debug rectangles (`developer` + F11).
    debug: bool,
    /// Shared `COM_Parse` line counter.
    parser_line: Cell<i32>,
    /// Disposed flag.
    disposed: bool,
}

impl UiRuntime {
    /// Create a runtime, resolving registrations and cinematics (`create`).
    ///
    /// `Promise -> sync`. Moves `options.definitions` into the runtime,
    /// leaving an empty placeholder behind.
    pub fn create(mut options: UiRuntimeOptions) -> Result<Self, ClientError> {
        let completed = matches!(
            options.definitions.registration,
            UiMenuRegistrationState::Completed { .. }
        );
        let mut pictures = HashMap::new();
        let mut sounds = HashMap::new();
        let mut models = HashMap::new();
        let mut cinematics = HashMap::new();
        let events = options.definitions.registration.events().to_vec();
        for event in &events {
            Self::resolve_registration(&mut options, completed, event, &mut pictures, &mut sounds, &mut models)?;
        }
        let menus = options.definitions.menus.clone();
        for menu in &menus {
            Self::cache_window_cinematic(&mut options, &menu.window(), &mut cinematics);
            for item in menu.items()? {
                Self::cache_window_cinematic(&mut options, &item.window(), &mut cinematics);
            }
        }
        let parser_line = options.source_parser.map(|parser| parser.line).unwrap_or(0);
        let mut definitions = UiMenuDefinitions::empty();
        std::mem::swap(&mut definitions, &mut options.definitions);
        let active_menu_count = definitions.menus.len();
        let bindings = match &mut options.context {
            UiRuntimeContext::Ui { bindings, .. } => load_bindings(bindings.as_mut())?,
            UiRuntimeContext::Cgame => empty_bindings(),
        };
        Ok(Self {
            options,
            definitions,
            active_menu_count,
            pictures,
            sounds,
            models,
            cinematics,
            open_stack: Vec::new(),
            bindings,
            display_cursor_x: 0.0,
            display_cursor_y: 0.0,
            real_time: 0,
            last_list_box_click_time: 0,
            editing_item: None,
            binding_item: None,
            waiting_for_key: false,
            capture: CaptureState::Idle,
            next_scroll_time: 0,
            next_scroll_adjust_time: 0,
            scroll_adjust_value: 0,
            debug: false,
            parser_line: Cell::new(parser_line),
            disposed: false,
        })
    }

    /// Resolve one registration event (`resolveRegistration`).
    fn resolve_registration(
        options: &mut UiRuntimeOptions,
        completed: bool,
        event: &UiMenuRegistrationEvent,
        pictures: &mut HashMap<String, Option<PictureAsset>>,
        sounds: &mut HashMap<String, Option<PcmSound>>,
        models: &mut HashMap<String, SceneModel>,
    ) -> Result<(), ClientError> {
        match event {
            UiMenuRegistrationEvent::Font { reference, .. } => {
                if !completed {
                    options
                        .resources
                        .register_font(reference.path.as_deref(), reference.point_size);
                }
            }
            UiMenuRegistrationEvent::Picture { reference, .. } => {
                let picture = if completed {
                    options.resources.registered_picture(reference.path.as_deref())
                } else {
                    options.resources.register_picture(reference.path.as_deref())
                };
                pictures.insert(resource_key(reference.path.as_deref()), picture);
            }
            UiMenuRegistrationEvent::Sound { reference, .. } => {
                let sound = if completed {
                    options.resources.registered_sound(reference.path.as_deref())
                } else {
                    options.resources.register_sound(reference.path.as_deref())
                };
                sounds.insert(resource_key(reference.path.as_deref()), sound);
            }
            UiMenuRegistrationEvent::Model { reference, .. } => {
                let model = if completed {
                    options.resources.registered_model(reference.path.as_deref())
                } else {
                    Some(options.resources.register_model(reference.path.as_deref()))
                };
                let Some(model) = model else {
                    return Err(bad_ui(format!(
                        "Completed UI model registration is missing {}",
                        reference.path.as_deref().unwrap_or("(null)")
                    )));
                };
                models.insert(resource_key(reference.path.as_deref()), model);
            }
        }
        Ok(())
    }

    /// Prepare a window cinematic (`cacheWindowCinematic`).
    fn cache_window_cinematic(
        options: &mut UiRuntimeOptions,
        window: &UiWindowDefinition,
        cinematics: &mut HashMap<String, UiCinematicAsset>,
    ) {
        if let Some(path) = window.cinematic().as_deref() {
            let asset = options.resources.prepare_cinematic(path);
            cinematics.insert(path.to_string(), asset);
        }
    }

    /// Drop menus, optionally clearing string state (`resetDefinitions`).
    pub fn reset_definitions(&mut self, reset: UiDefinitionReset) -> Result<(), ClientError> {
        self.opened()?;
        self.active_menu_count = 0;
        if reset == UiDefinitionReset::Strings {
            self.open_stack.clear();
            if matches!(self.options.context, UiRuntimeContext::Ui { .. }) {
                self.reload_bindings()?;
            }
        }
        Ok(())
    }

    /// Accept one completed registration (`acceptMenuRegistration`).
    pub fn accept_menu_registration(&mut self, event: &UiMenuRegistrationEvent) -> Result<(), ClientError> {
        self.opened()?;
        let key = resource_key(event.path());
        match event {
            UiMenuRegistrationEvent::Font { .. } => {}
            UiMenuRegistrationEvent::Picture { reference, .. } => {
                self.pictures.insert(
                    key,
                    self.options.resources.registered_picture(reference.path.as_deref()),
                );
            }
            UiMenuRegistrationEvent::Sound { reference, .. } => {
                self.sounds
                    .insert(key, self.options.resources.registered_sound(reference.path.as_deref()));
            }
            UiMenuRegistrationEvent::Model { reference, .. } => {
                let model = self.options.resources.registered_model(reference.path.as_deref());
                let Some(model) = model else {
                    return Err(bad_ui(format!(
                        "Completed UI model registration is missing {}",
                        reference.path.as_deref().unwrap_or("(null)")
                    )));
                };
                self.models.insert(key, model);
            }
        }
        Ok(())
    }

    /// Publish one global asset (`publishMenuAsset`).
    pub fn publish_menu_asset(&mut self, event: UiMenuAssetPublication) -> Result<(), ClientError> {
        self.opened()?;
        match event {
            UiMenuAssetPublication::CursorStr(_) => {}
            UiMenuAssetPublication::FontRegistered(value) => {
                self.definitions.font_registered = value;
            }
            UiMenuAssetPublication::ShadowColorComponent { component, value } => {
                let color = &mut self.definitions.assets.shadow_color;
                match component {
                    UiColorComponent::X => color.x = value,
                    UiColorComponent::Y => color.y = value,
                    UiColorComponent::Z => color.z = value,
                    UiColorComponent::W => color.w = value,
                }
            }
            UiMenuAssetPublication::TextFont(value) => {
                self.definitions.assets.text_font = Some(value);
            }
            UiMenuAssetPublication::SmallFont(value) => {
                self.definitions.assets.small_font = Some(value);
            }
            UiMenuAssetPublication::BigFont(value) => {
                self.definitions.assets.big_font = Some(value);
            }
            UiMenuAssetPublication::Cursor(value) => {
                self.definitions.assets.cursor = Some(value);
            }
            UiMenuAssetPublication::GradientBar(value) => {
                self.definitions.assets.gradient_bar = Some(value);
            }
            UiMenuAssetPublication::MenuEnterSound(value) => {
                self.definitions.assets.menu_enter_sound = Some(value);
            }
            UiMenuAssetPublication::MenuExitSound(value) => {
                self.definitions.assets.menu_exit_sound = Some(value);
            }
            UiMenuAssetPublication::MenuBuzzSound(value) => {
                self.definitions.assets.menu_buzz_sound = Some(value);
            }
            UiMenuAssetPublication::ItemFocusSound(value) => {
                self.definitions.assets.item_focus_sound = Some(value);
            }
            UiMenuAssetPublication::FadeClamp(value) => {
                self.definitions.assets.fade_clamp = value;
            }
            UiMenuAssetPublication::FadeCycle(value) => {
                self.definitions.assets.fade_cycle = value;
            }
            UiMenuAssetPublication::FadeAmount(value) => {
                self.definitions.assets.fade_amount = value;
            }
            UiMenuAssetPublication::ShadowX(value) => {
                self.definitions.assets.shadow_x = value;
            }
            UiMenuAssetPublication::ShadowY(value) => {
                self.definitions.assets.shadow_y = value;
            }
            UiMenuAssetPublication::ShadowFadeClamp(value) => {
                self.definitions.assets.shadow_fade_clamp = value;
            }
        }
        Ok(())
    }

    /// Append a menu definition (`appendMenu`).
    ///
    /// `Promise -> sync`.
    pub fn append_menu(
        &mut self,
        definition: UiMenuDefinition,
        memory: &UiMenuMemoryOwnership,
    ) -> Result<(), ClientError> {
        self.opened()?;
        self.assert_menu_memory(memory)?;
        if self.active_menu_count >= MAX_UI_MENUS {
            return Err(bad_ui("UI menu publication exceeds the source static menu array"));
        }
        let index = self.active_menu_count;
        let cinematic = definition.window().cinematic();
        if let Some(path) = cinematic.as_deref() {
            let asset = self.options.resources.prepare_cinematic(path);
            self.cinematics.insert(path.to_string(), asset);
        }
        for item in definition.items()? {
            if let Some(path) = item.window().cinematic().as_deref() {
                let asset = self.options.resources.prepare_cinematic(path);
                self.cinematics.insert(path.to_string(), asset);
            }
        }
        if self.active_menu_count != index {
            return Err(bad_ui("UI menu publication was interrupted by another menu load"));
        }
        if index < self.definitions.menus.len() {
            self.definitions.menus[index] = definition;
        } else {
            self.definitions.menus.push(definition);
        }
        self.active_menu_count += 1;
        Ok(())
    }

    /// Assert reload memory ownership (`assertMenuMemory`).
    pub fn assert_menu_memory(&self, next: &UiMenuMemoryOwnership) -> Result<(), ClientError> {
        self.opened()?;
        if self.definitions.memory != *next {
            return Err(bad_ui("UI reload cannot replace its source memory owner"));
        }
        Ok(())
    }

    /// Reload all definitions (`reloadDefinitions`).
    ///
    /// `Promise -> sync`. Slots beyond the new menu list keep their stale
    /// definitions, like the donor's untouched static tail.
    pub fn reload_definitions(&mut self, definitions: UiMenuDefinitions) -> Result<(), ClientError> {
        self.opened()?;
        self.assert_menu_memory(&definitions.memory)?;
        let completed = matches!(definitions.registration, UiMenuRegistrationState::Completed { .. });
        let mut pictures = std::mem::take(&mut self.pictures);
        let mut sounds = std::mem::take(&mut self.sounds);
        let mut models = std::mem::take(&mut self.models);
        for event in definitions.registration.events() {
            Self::resolve_registration(
                &mut self.options,
                completed,
                event,
                &mut pictures,
                &mut sounds,
                &mut models,
            )?;
        }
        self.pictures = pictures;
        self.sounds = sounds;
        self.models = models;
        for menu in &definitions.menus {
            if let Some(path) = menu.window().cinematic().as_deref() {
                let asset = self.options.resources.prepare_cinematic(path);
                self.cinematics.insert(path.to_string(), asset);
            }
            for item in menu.items()? {
                if let Some(path) = item.window().cinematic().as_deref() {
                    let asset = self.options.resources.prepare_cinematic(path);
                    self.cinematics.insert(path.to_string(), asset);
                }
            }
        }
        let mut merged = definitions;
        let active_menu_count = merged.menus.len();
        let old = std::mem::take(&mut self.definitions.menus);
        if old.len() > merged.menus.len() {
            merged.menus.extend(old.into_iter().skip(merged.menus.len()));
        }
        self.active_menu_count = active_menu_count;
        self.definitions = merged;
        Ok(())
    }

    /// Activate (open) a menu (`activate`).
    ///
    /// `Promise -> sync`. A null selector clears focus everywhere and matches
    /// nothing, like the donor.
    pub fn activate(&mut self, mut selector: UiMenuSelector) -> Result<bool, ClientError> {
        self.opened()?;
        let focus = self.focused_menu();
        let mut activated = false;
        for index in 0..self.active_menu_count {
            let unnamed = self.definitions.menus[index].window().name().is_none();
            let requested = if unnamed || matches!(selector, UiMenuSelector::Null) {
                None
            } else {
                Some(match &mut selector {
                    UiMenuSelector::Name(name) => quake_string(name, None)?,
                    UiMenuSelector::Null => String::new(),
                    UiMenuSelector::Resolve(resolve) => quake_string(&resolve(), None)?,
                })
            };
            let matches = match requested.as_deref() {
                Some(name) => equal_name(self.definitions.menus[index].window().name().as_deref(), name),
                None => false,
            };
            if matches {
                activated = true;
                self.activate_menu(index)?;
                if focus.is_some() && self.open_stack.len() < MAX_OPEN_MENUS {
                    if let Some(focused) = focus {
                        self.open_stack.push(focused);
                    }
                }
            } else {
                self.definitions.menus[index]
                    .window()
                    .set_flags(self.definitions.menus[index].window().flags() & (!UiWindowFlag::HAS_FOCUS));
            }
        }
        self.close_cinematics();
        Ok(activated)
    }

    /// Show a menu (`show`).
    ///
    /// `Promise -> sync`.
    pub fn show(&mut self, name: &str) -> Result<bool, ClientError> {
        self.opened()?;
        let menu = self.find_menu(name)?;
        let Some(index) = menu else {
            return Ok(false);
        };
        self.activate_menu(index)?;
        Ok(true)
    }

    /// Close a menu (`close`).
    ///
    /// `Promise -> sync`.
    pub fn close(&mut self, name: &str) -> Result<bool, ClientError> {
        self.opened()?;
        let menu = self.find_menu(name)?;
        let Some(index) = menu else {
            return Ok(false);
        };
        self.close_menu(index)?;
        Ok(true)
    }

    /// Close all menus (`closeAll`).
    ///
    /// `Promise -> sync`.
    pub fn close_all(&mut self) -> Result<(), ClientError> {
        self.opened()?;
        for index in 0..self.active_menu_count {
            self.close_menu(index)?;
        }
        Ok(())
    }

    /// Whether any visible menu is full-screen (`anyFullScreenVisible`).
    pub fn any_full_screen_visible(&self) -> Result<bool, ClientError> {
        self.opened()?;
        Ok(self.definitions.menus[..self.active_menu_count]
            .iter()
            .any(|menu| menu.window().flags() & UiWindowFlag::VISIBLE != 0 && menu.full_screen() != 0))
    }

    /// Route pointer motion (`pointerMove`).
    ///
    /// `Promise -> sync`.
    pub fn pointer_move(&mut self, x: f32, y: f32) -> Result<bool, ClientError> {
        self.opened()?;
        if !x.is_finite() || !y.is_finite() {
            return Err(bad_ui("UI pointer coordinates must be finite"));
        }
        if let Some(focused) = self.focused_menu() {
            if self.definitions.menus[focused].window().flags() & UiWindowFlag::POPUP != 0 {
                self.mouse_move_menu(focused, x, y)?;
                return Ok(true);
            }
        }
        for index in 0..self.active_menu_count {
            self.mouse_move_menu(index, x, y)?;
        }
        Ok(true)
    }

    /// Set the display cursor (`setDisplayCursor`).
    pub fn set_display_cursor(&mut self, x: f32, y: f32) -> Result<(), ClientError> {
        self.opened()?;
        if !x.is_finite() || !y.is_finite() {
            return Err(bad_ui("UI display cursor coordinates must be finite"));
        }
        self.display_cursor_x = x;
        self.display_cursor_y = y;
        Ok(())
    }

    /// Handle a key or character event (`handleKey`).
    ///
    /// `Promise -> sync`.
    pub fn handle_key(&mut self, event: UiKeyEvent, x: f32, y: f32) -> Result<bool, ClientError> {
        self.opened()?;
        let (code, down, character) = match event {
            UiKeyEvent::Key { code, down } => (code, down, false),
            UiKeyEvent::Character { code } => (code, true, true),
        };
        if !(0..=0x7fff).contains(&code) {
            return Err(bad_ui("UI key code must be an integer in 0..32767"));
        }
        if character && code > 255 {
            return Err(bad_ui("UI character code must fit one byte"));
        }
        if !x.is_finite() || !y.is_finite() {
            return Err(bad_ui("UI key coordinates must be finite"));
        }
        let menu = self.menu_at(x, y).or_else(|| self.focused_menu());
        let Some(index) = menu else {
            return Ok(false);
        };
        let key = if character { code | KEY_CHAR_FLAG } else { code };
        self.handle_menu_key(index, key, down)
    }

    /// Focused-menu handle (`focusedMenuHandle`).
    pub fn focused_menu_handle(&self) -> Result<Option<UiCapturedMenu>, ClientError> {
        self.opened()?;
        Ok(self.focused_menu().map(|index| UiCapturedMenu { index }))
    }

    /// Handle a key for a captured menu (`handleCapturedKey`).
    ///
    /// `Promise -> sync`.
    pub fn handle_captured_key(&mut self, handle: UiCapturedMenu, event: UiKeyEvent) -> Result<bool, ClientError> {
        self.opened()?;
        let index = self.captured_menu(handle)?;
        let (code, down, character) = match event {
            UiKeyEvent::Key { code, down } => (code, down, false),
            UiKeyEvent::Character { code } => (code, true, true),
        };
        if !(0..=0x7fff).contains(&code) {
            return Err(bad_ui("UI key code must be an integer in 0..32767"));
        }
        if character && code > 255 {
            return Err(bad_ui("UI character code must fit one byte"));
        }
        let key = if character { code | KEY_CHAR_FLAG } else { code };
        self.handle_menu_key(index, key, down)
    }

    /// Run capture timers and paint all menus (`frame`).
    ///
    /// `Promise -> sync`.
    pub fn frame(&mut self, mut frame: UiRuntimeFrame, frames_per_second: f32) -> Result<(), ClientError> {
        self.opened()?;
        self.set_frame_time(&frame)?;
        self.run_capture()?;
        for index in 0..self.active_menu_count {
            self.paint_menu(index, &mut frame.draw, false)?;
        }
        if self.debug {
            let text = game_format("fps: %f", &[GameFormatArg::Float(frames_per_second)])?;
            text_paint(
                &mut frame.draw,
                &self.options.fonts,
                &TextPaintOptions {
                    x: 5.0,
                    y: 25.0,
                    scale: 0.5,
                    color: Vec4 {
                        x: 1.0,
                        y: 1.0,
                        z: 1.0,
                        w: 1.0,
                    },
                    text: &text,
                    adjust: 0.0,
                    limit: 0,
                    style: 0,
                },
            )?;
        }
        Ok(())
    }

    /// Cache reached sounds and window movies (`cacheAll`).
    ///
    /// `Promise -> sync`. Registers reached sounds and plays/stops each
    /// window movie without changing its state.
    pub fn cache_all(&mut self) -> Result<(), ClientError> {
        self.opened()?;
        for menu in 0..self.active_menu_count {
            let paths: Vec<Option<String>> = {
                let definition = &self.definitions.menus[menu];
                let mut paths = vec![definition.window().cinematic()];
                for index in 0..definition.item_count().max(0) as usize {
                    let Ok(item) = definition.item_view(index) else {
                        continue;
                    };
                    paths.push(item.window().cinematic());
                }
                paths
            };
            for path in paths.into_iter().flatten() {
                let asset = self.cinematic_asset(&path);
                let instance = self.options.cinematics.play(
                    &asset,
                    &UiRect {
                        x: 0.0,
                        y: 0.0,
                        width: 0.0,
                        height: 0.0,
                    },
                );
                self.options
                    .cinematics
                    .stop(instance.map(|found| found.handle).unwrap_or(-1));
            }
            let sound_loop = self.definitions.menus[menu].sound_loop();
            if let Some(sound) = sound_loop.as_ref().and_then(|found| found.path.as_deref()) {
                if !quake_string(sound, None)?.is_empty() {
                    self.options.resources.register_sound(Some(sound));
                }
            }
        }
        Ok(())
    }

    /// Set the display time (`setDisplayTime`).
    pub fn set_display_time(&mut self, time: i32) -> Result<(), ClientError> {
        self.opened()?;
        self.real_time = time;
        Ok(())
    }

    /// Paint a named menu (`paintNamed`).
    ///
    /// `Promise -> sync`.
    pub fn paint_named(&mut self, name: &str, mut frame: UiRuntimeFrame, force: bool) -> Result<bool, ClientError> {
        self.opened()?;
        self.set_frame_time(&frame)?;
        let menu = self.find_menu(name)?;
        let Some(index) = menu else {
            return Ok(false);
        };
        self.paint_menu(index, &mut frame.draw, force)?;
        Ok(true)
    }

    /// Paint a captured menu (`paintCaptured`).
    ///
    /// `Promise -> sync`.
    pub fn paint_captured(
        &mut self,
        handle: UiCapturedMenu,
        mut frame: UiRuntimeFrame,
        force: bool,
    ) -> Result<(), ClientError> {
        self.opened()?;
        self.set_frame_time(&frame)?;
        let index = self.captured_menu(handle)?;
        self.paint_menu(index, &mut frame.draw, force)
    }

    /// Clear a menu's forced flag (`clearForced`).
    pub fn clear_forced(&mut self, name: &str) -> Result<bool, ClientError> {
        self.opened()?;
        let menu = self.find_menu(name)?;
        let Some(index) = menu else {
            return Ok(false);
        };
        self.definitions.menus[index]
            .window()
            .set_flags(self.definitions.menus[index].window().flags() & (!UiWindowFlag::FORCED));
        Ok(true)
    }

    /// Clear a captured menu's forced flag (`clearCapturedForced`).
    pub fn clear_captured_forced(&mut self, handle: UiCapturedMenu) -> Result<(), ClientError> {
        self.opened()?;
        let index = self.captured_menu(handle)?;
        self.definitions.menus[index]
            .window()
            .set_flags(self.definitions.menus[index].window().flags() & (!UiWindowFlag::FORCED));
        Ok(())
    }

    /// Look up a menu handle by name (`menuHandle`).
    pub fn menu_handle(&self, name: &str) -> Result<Option<UiCapturedMenu>, ClientError> {
        self.opened()?;
        Ok(self.find_menu(name)?.map(|index| UiCapturedMenu { index }))
    }

    /// Capture the menu under a point (`captureMenu`).
    pub fn capture_menu(&self, x: f32, y: f32) -> Result<Option<UiCapturedMenu>, ClientError> {
        self.opened()?;
        if !x.is_finite() || !y.is_finite() {
            return Err(bad_ui("UI capture coordinates must be finite"));
        }
        Ok(self.menu_at(x, y).map(|index| UiCapturedMenu { index }))
    }

    /// Hit-test a captured menu (`hitTestMenu`).
    pub fn hit_test_menu(
        &self,
        handle: UiCapturedMenu,
        x: f32,
        y: f32,
    ) -> Result<Option<UiItemDefinition>, ClientError> {
        self.opened()?;
        let index = self.captured_menu(handle)?;
        let menu = &self.definitions.menus[index];
        for slot in 0..menu.item_count().max(0) as usize {
            let Ok(item) = menu.item_view(slot) else {
                continue;
            };
            if rect_contains(&item.window().rect().snapshot(), x, y) {
                return Ok(Some(item));
            }
        }
        Ok(None)
    }

    /// Set an item's mouse-over flag (`setItemMouseOver`).
    ///
    /// Mutates the passed definition, like the donor; callers holding
    /// [`UiRuntime::hit_test_menu`] clones must write the result back through
    /// their own copy.
    pub fn set_item_mouse_over(&self, item: Option<&mut UiItemDefinition>, focused: bool) -> Result<(), ClientError> {
        self.opened()?;
        if let Some(definition) = item {
            if focused {
                definition
                    .window()
                    .set_flags(definition.window().flags() | (UiWindowFlag::MOUSE_OVER));
            } else {
                definition
                    .window()
                    .set_flags(definition.window().flags() & (!UiWindowFlag::MOUSE_OVER));
            }
        }
        Ok(())
    }

    /// Paint an item image (`paintItemImage`).
    pub fn paint_item_image(&self, item: Option<&UiItemDefinition>, draw: &mut Draw2D) -> Result<(), ClientError> {
        self.opened()?;
        let Some(definition) = item else {
            return Ok(());
        };
        let rect = &definition.window().rect();
        let picture = match definition.asset_handle() {
            Some(handle) if handle != 0 => self
                .options
                .resources
                .picture_for_handle(handle)?
                .unwrap_or(self.options.zero_picture),
            None => match definition.asset().as_ref() {
                None => self.options.zero_picture,
                Some(UiMenuResource::Shader(shader)) => self
                    .picture(shader.path.as_deref())
                    .unwrap_or(self.options.zero_picture),
                Some(UiMenuResource::Model(_)) => {
                    return Err(bad_ui(
                        "Item_Image_Paint needs the source numeric handle for a model asset",
                    ));
                }
                Some(UiMenuResource::Sound(_)) => self.options.zero_picture,
            },
            Some(_) => self.options.zero_picture,
        };
        draw.draw_handle_pic(
            Rect {
                x: f(rect.x() + 1.0),
                y: f(rect.y() + 1.0),
                width: f(rect.width() - 2.0),
                height: f(rect.height() - 2.0),
            },
            picture,
        );
        Ok(())
    }

    /// Move a captured menu (`moveCapturedMenu`).
    pub fn move_captured_menu(
        &mut self,
        handle: UiCapturedMenu,
        delta_x: f32,
        delta_y: f32,
    ) -> Result<(), ClientError> {
        self.opened()?;
        if !delta_x.is_finite() || !delta_y.is_finite() {
            return Err(bad_ui("UI captured-menu delta must be finite"));
        }
        let index = self.captured_menu(handle)?;
        {
            let menu = &mut self.definitions.menus[index];
            menu.window().rect().set_x(f(menu.window().rect().x() + delta_x));
            menu.window().rect().set_y(f(menu.window().rect().y() + delta_y));
        }
        let (mut x, mut y, border, border_size) = {
            let menu = &self.definitions.menus[index];
            (
                menu.window().rect().x(),
                menu.window().rect().y(),
                menu.window().border(),
                menu.window().border_size(),
            )
        };
        if border != 0 {
            x = f(x + border_size);
            y = f(y + border_size);
        }
        for item in 0..self.definitions.menus[index].item_count().max(0) as usize {
            set_item_screen_coords(&self.menu_item(index, item)?, x, y);
        }
        Ok(())
    }

    /// Set a feeder selection (`setFeederSelection`).
    ///
    /// `Promise -> sync`.
    pub fn set_feeder_selection(
        &mut self,
        feeder: f32,
        index: i32,
        menu_name: Option<&str>,
    ) -> Result<(), ClientError> {
        self.opened()?;
        if !feeder.is_finite() {
            return Err(bad_ui("Invalid feeder selection"));
        }
        let menu = match menu_name {
            Some(name) => self.find_menu(name)?,
            None => self.focused_menu(),
        };
        let Some(menu) = menu else {
            return Ok(());
        };
        self.select_feeder(menu, feeder, index)
    }

    /// Set a captured feeder selection (`setCapturedFeederSelection`).
    ///
    /// `Promise -> sync`.
    pub fn set_captured_feeder_selection(
        &mut self,
        handle: UiCapturedMenu,
        feeder: f32,
        index: i32,
    ) -> Result<(), ClientError> {
        self.opened()?;
        if !feeder.is_finite() {
            return Err(bad_ui("Invalid feeder selection"));
        }
        let menu = self.captured_menu(handle)?;
        self.select_feeder(menu, feeder, index)
    }

    /// Select a feeder row (`selectFeeder`).
    ///
    /// `Promise -> sync`.
    fn select_feeder(&mut self, menu: usize, feeder: f32, index: i32) -> Result<(), ClientError> {
        let definition = &self.definitions.menus[menu];
        let mut item = None;
        for slot in 0..definition.item_count().max(0) as usize {
            let Ok(candidate) = definition.item_view(slot) else {
                continue;
            };
            if candidate.special() == feeder {
                item = Some(slot);
                break;
            }
        }
        let Some(item) = item else {
            return Ok(());
        };
        if index == 0 {
            let list = self.definitions.menus[menu]
                .item_at(item)
                .ok_or_else(|| bad_ui("UI menu item index is out of range"))?
                .list_data();
            let Some(list) = list else {
                return Err(bad_ui("Menu_SetFeederSelection dereferences NULL list data"));
            };
            list.set_cursor_position(0);
            list.set_start_position(0);
        }
        self.menu_item(menu, item)?.set_cursor_position(index);
        let special = self.menu_item(menu, item)?.special();
        let cursor = self.menu_item(menu, item)?.cursor_position();
        self.options.feeder.select(special, cursor);
        Ok(())
    }

    /// Scroll a feeder (`scrollFeeder`).
    ///
    /// `Promise -> sync`.
    pub fn scroll_feeder(&mut self, feeder: f32, down: bool, menu_name: Option<&str>) -> Result<(), ClientError> {
        self.opened()?;
        let menu = match menu_name {
            Some(name) => self.find_menu(name)?,
            None => self.focused_menu(),
        };
        let Some(menu) = menu else {
            return Ok(());
        };
        let definition = &self.definitions.menus[menu];
        let mut item = None;
        for slot in 0..definition.item_count().max(0) as usize {
            let Ok(candidate) = definition.item_view(slot) else {
                continue;
            };
            if candidate.special() == feeder {
                item = Some(slot);
                break;
            }
        }
        if let Some(item) = item {
            let key = if down { KeyCode::Down as i32 } else { KeyCode::Up as i32 };
            self.handle_list_key(
                ItemState {
                    menu_index: menu,
                    item_index: item,
                },
                key,
                true,
            )?;
        }
        Ok(())
    }

    /// Scroll a captured feeder (`scrollCapturedFeeder`).
    ///
    /// `Promise -> sync`.
    pub fn scroll_captured_feeder(
        &mut self,
        handle: UiCapturedMenu,
        feeder: f32,
        down: bool,
    ) -> Result<(), ClientError> {
        self.opened()?;
        let menu = self.captured_menu(handle)?;
        let definition = &self.definitions.menus[menu];
        let mut item = None;
        for slot in 0..definition.item_count().max(0) as usize {
            let Ok(candidate) = definition.item_view(slot) else {
                continue;
            };
            if candidate.special() == feeder {
                item = Some(slot);
                break;
            }
        }
        if let Some(item) = item {
            let key = if down { KeyCode::Down as i32 } else { KeyCode::Up as i32 };
            self.handle_list_key(
                ItemState {
                    menu_index: menu,
                    item_index: item,
                },
                key,
                true,
            )?;
        }
        Ok(())
    }

    /// Whether a binding key is awaited (`bindingPending`).
    pub fn binding_pending(&self) -> Result<bool, ClientError> {
        self.opened()?;
        Ok(self.waiting_for_key)
    }

    /// Reload bindings from the host (`reloadBindings`).
    pub fn reload_bindings(&mut self) -> Result<(), ClientError> {
        self.opened()?;
        let loaded = match &mut self.options.context {
            UiRuntimeContext::Ui { bindings, .. } => load_bindings(bindings.as_mut())?,
            UiRuntimeContext::Cgame => {
                return Err(bad_ui("UI reload bindings is unavailable in cgame context"));
            }
        };
        for (index, source) in loaded.into_iter().enumerate() {
            if let Some(target) = self.bindings.get_mut(index) {
                target.first = source.first;
                target.second = source.second;
            }
        }
        Ok(())
    }

    /// Clear all bindings (`resetBindings`).
    pub fn reset_bindings(&mut self) -> Result<(), ClientError> {
        self.opened()?;
        for binding in &mut self.bindings {
            binding.first = NO_BINDING;
            binding.second = NO_BINDING;
        }
        Ok(())
    }

    /// Write bindings to the host (`applyBindings`).
    pub fn apply_bindings(&mut self) -> Result<(), ClientError> {
        self.opened()?;
        self.write_bindings()
    }

    /// Pointer cursor shape (`cursorType`).
    pub fn cursor_type(&self, x: f32, y: f32) -> Result<UiCursorType, ClientError> {
        self.opened()?;
        for menu in &self.definitions.menus[..self.active_menu_count] {
            let sizer = UiRect {
                x: f(menu.window().rect().x() - 3.0),
                y: f(menu.window().rect().y() - 3.0),
                width: 7.0,
                height: 7.0,
            };
            if rect_contains(&sizer, x, y) {
                return Ok(UiCursorType::Sizer);
            }
        }
        Ok(UiCursorType::Arrow)
    }

    /// Run a menu script (`runMenuScript`).
    ///
    /// `Promise -> sync`.
    pub fn run_menu_script(&mut self, menu_name: &str, script: &UiScript) -> Result<(), ClientError> {
        self.opened()?;
        let menu = self.find_menu(menu_name)?;
        let Some(menu) = menu else {
            return Err(bad_ui(format!("Unknown UI menu {menu_name}")));
        };
        let script = script.clone();
        self.run_script(ScriptOwner { menu, item: None }, &script)
    }

    /// Run an item script (`runItemScript`).
    ///
    /// `Promise -> sync`.
    pub fn run_item_script(&mut self, menu_name: &str, item_name: &str, script: &UiScript) -> Result<(), ClientError> {
        self.opened()?;
        let menu = self.find_menu(menu_name)?;
        let Some(menu) = menu else {
            return Err(bad_ui(format!("Unknown UI menu {menu_name}")));
        };
        let definition = &self.definitions.menus[menu];
        let mut found = None;
        for slot in 0..definition.item_count().max(0) as usize {
            let Ok(candidate) = definition.item_view(slot) else {
                continue;
            };
            if equal_name(candidate.window().name().as_deref(), item_name) {
                found = Some(slot);
                break;
            }
        }
        let Some(found) = found else {
            return Err(bad_ui(format!("Unknown UI item {item_name} in {menu_name}")));
        };
        let script = script.clone();
        self.run_script(
            ScriptOwner {
                menu,
                item: Some(ItemState {
                    menu_index: menu,
                    item_index: found,
                }),
            },
            &script,
        )
    }

    /// Active menu count (`menuCount`).
    pub fn menu_count(&self) -> Result<usize, ClientError> {
        self.opened()?;
        Ok(self.active_menu_count)
    }

    /// Fail when disposed (`opened`).
    fn opened(&self) -> Result<(), ClientError> {
        if self.disposed {
            return Err(bad_ui("UI runtime is disposed"));
        }
        Ok(())
    }

    /// Resolve a captured handle to its menu slot.
    fn captured_menu(&self, handle: UiCapturedMenu) -> Result<usize, ClientError> {
        if handle.index < self.definitions.menus.len() {
            Ok(handle.index)
        } else {
            Err(bad_ui("UI captured-menu handle does not belong to this runtime"))
        }
    }

    /// Find an active menu by name (`findMenu`).
    fn find_menu(&self, name: &str) -> Result<Option<usize>, ClientError> {
        let requested = quake_string(name, None)?;
        Ok(self.definitions.menus[..self.active_menu_count]
            .iter()
            .position(|menu| equal_name(menu.window().name().as_deref(), &requested)))
    }

    /// Focused visible menu (`focusedMenu`).
    fn focused_menu(&self) -> Option<usize> {
        self.definitions.menus[..self.active_menu_count]
            .iter()
            .position(|menu| {
                menu.window().flags() & UiWindowFlag::HAS_FOCUS != 0
                    && menu.window().flags() & UiWindowFlag::VISIBLE != 0
            })
    }

    /// Menu under a point (`menuAt`).
    fn menu_at(&self, x: f32, y: f32) -> Option<usize> {
        self.definitions.menus[..self.active_menu_count]
            .iter()
            .position(|menu| rect_contains(&menu.window().rect().snapshot(), x, y))
    }

    /// Borrow a menu.
    fn menu(&self, index: usize) -> &UiMenuDefinition {
        &self.definitions.menus[index]
    }

    /// Borrow an item (cheap live view over shared menu memory).
    fn item(&self, item: ItemState) -> Result<UiItemDefinition, ClientError> {
        self.definitions.menus[item.menu_index]
            .item_at(item.item_index)
            .ok_or_else(|| bad_ui("UI menu item index is out of range"))
    }

    /// Borrow an item by menu/item slot (cheap live view over shared menu memory).
    fn menu_item(&self, menu: usize, item: usize) -> Result<UiItemDefinition, ClientError> {
        self.item(ItemState {
            menu_index: menu,
            item_index: item,
        })
    }

    /// Parent menu slot (`parentMenu`).
    fn parent_menu(&self, item: ItemState) -> Result<Option<usize>, ClientError> {
        let Some(parent) = self.item(item)?.parent() else {
            return Ok(None);
        };
        let slot = self.definitions.menus.iter().position(|menu| menu.same_record(&parent));
        let Some(slot) = slot else {
            return Err(bad_ui("UI item belongs to a missing menu"));
        };
        Ok(Some(slot))
    }

    /// Parent menu slot, failing when absent (`menuFor`).
    fn menu_for(&self, item: ItemState) -> Result<usize, ClientError> {
        self.parent_menu(item)?
            .ok_or_else(|| bad_ui("UI item dereferences a NULL parent menu"))
    }

    /// Item indices matching a group name (`matching`).
    fn matching_indices(&self, menu: usize, name: Option<&str>) -> Vec<usize> {
        let Some(name) = name else {
            return Vec::new();
        };
        let definition = &self.definitions.menus[menu];
        let mut indices = Vec::new();
        for index in 0..definition.item_count().max(0) as usize {
            let Ok(item) = definition.item_view(index) else {
                continue;
            };
            if matches_item(&item, name) {
                indices.push(index);
            }
        }
        indices
    }

    /// Set a cvar with source-text validation (`setCvar`).
    fn set_cvar(&mut self, name: Option<&str>, value: Option<&str>, force: bool) {
        let text = name.and_then(|name| source_command_text(name).ok());
        let invalid = match text.as_deref() {
            None => true,
            Some(text) => text.contains('\\') || text.contains('"') || text.contains(';'),
        };
        let key = if invalid {
            if let Some(print) = self.options.print.as_mut() {
                print(&format!(
                    "invalid cvar name string: {}\n",
                    text.as_deref().unwrap_or("(null)")
                ));
            }
            "BADNAME".to_string()
        } else {
            text.unwrap_or_else(|| "BADNAME".to_string())
        };
        match value {
            None => self.options.cvars.reset(&key, force),
            Some(value) => self.options.cvars.set(&key, value, force),
        }
    }

    /// Diagnostic print hook.
    fn print(&mut self, text: &str) {
        if let Some(print) = self.options.print.as_mut() {
            print(text);
        }
    }

    /// Apply a frame's time (`setFrameTime`).
    fn set_frame_time(&mut self, frame: &UiRuntimeFrame) -> Result<(), ClientError> {
        let _ = frame.frame_time;
        self.set_display_time(frame.time)
    }

    /// Binding host, failing in cgame contexts.
    fn binding_host(&mut self, operation: &str) -> Result<&mut dyn UiRuntimeBindings, ClientError> {
        match &mut self.options.context {
            UiRuntimeContext::Ui { bindings, .. } => Ok(bindings.as_mut()),
            UiRuntimeContext::Cgame => Err(bad_ui(format!("UI {operation} is unavailable in cgame context"))),
        }
    }

    /// Find a binding row by command name (`bindingByName`).
    fn binding_by_name(&self, name: Option<&str>) -> Option<usize> {
        let name = name?;
        self.bindings
            .iter()
            .position(|binding| equal_name(Some(&binding.command), name))
    }

    /// Write bindings to the host and restart input (`writeBindings`).
    fn write_bindings(&mut self) -> Result<(), ClientError> {
        match &mut self.options.context {
            UiRuntimeContext::Ui { bindings, .. } => {
                for binding in &self.bindings.clone() {
                    if binding.first != NO_BINDING {
                        bindings.set_binding(binding.first, &binding.command);
                        if binding.second != NO_BINDING {
                            bindings.set_binding(binding.second, &binding.command);
                        }
                    }
                }
            }
            UiRuntimeContext::Cgame => {
                return Err(bad_ui("UI write bindings is unavailable in cgame context"));
            }
        }
        let context = self.options.command_context.clone();
        self.options.commands.append("in_restart\n", &context);
        Ok(())
    }

    /// Execute console text (`executeText`).
    fn execute_text(&mut self, text: &str) -> Result<(), ClientError> {
        if matches!(self.options.context, UiRuntimeContext::Cgame) {
            return Err(bad_ui("UI executeText is unavailable in cgame context"));
        }
        let context = self.options.command_context.clone();
        self.options.commands.append(text, &context);
        Ok(())
    }

    /// Look up a resolved picture.
    fn picture(&self, path: Option<&str>) -> Option<PictureAsset> {
        self.pictures.get(&resource_key(path)).copied().flatten()
    }

    /// Look up a resolved sound.
    fn sound(&self, path: Option<&str>) -> Option<PcmSound> {
        self.sounds.get(&resource_key(path)).cloned().flatten()
    }

    /// Resolve a window background (`windowPicture`).
    fn window_picture(&self, window: &UiWindowDefinition) -> Result<Option<PictureAsset>, ClientError> {
        match window.background_handle() {
            Some(handle) => {
                if handle == 0 {
                    Ok(None)
                } else {
                    Ok(self.options.resources.picture_for_handle(handle)?)
                }
            }
            None => Ok(
                match window.background().as_ref().and_then(|found| found.path.as_deref()) {
                    Some(path) => self.picture(Some(path)),
                    None => None,
                },
            ),
        }
    }

    /// Whether a window has a background (`hasWindowBackground`).
    fn has_window_background(&self, window: &UiWindowDefinition) -> Result<bool, ClientError> {
        match window.background_handle() {
            Some(handle) => Ok(handle != 0),
            None => Ok(self.window_picture(window)?.is_some()),
        }
    }

    /// Resolve a background or the zero picture (`backgroundOrZero`).
    fn background_or_zero(&self, window: &UiWindowDefinition) -> Result<PictureAsset, ClientError> {
        Ok(self.window_background(window)?.unwrap_or(self.options.zero_picture))
    }

    /// Resolve a window background eagerly (`windowBackground`).
    ///
    /// The donor returns a lazy closure for numeric handles; this port
    /// resolves at paint time (see module docs).
    fn window_background(&self, window: &UiWindowDefinition) -> Result<Option<PictureAsset>, ClientError> {
        match window.background_handle() {
            Some(handle) if handle != 0 => Ok(self.options.resources.picture_for_handle(handle)?),
            _ => self.window_picture(window),
        }
    }

    /// Resolve a widget picture through numeric handles (`widgetPicture`).
    fn widget_picture(&mut self, picture: PictureAsset) -> Result<PictureAsset, ClientError> {
        if self.options.resources.handle_kind() == UiHandleKind::Diagnostic {
            return Ok(picture);
        }
        let handle = self.options.resources.picture_handle(Some(picture))?;
        Ok(self
            .options
            .resources
            .picture_for_handle(handle)?
            .unwrap_or(self.options.zero_picture))
    }

    /// Activate a menu slot (`activateMenu`).
    ///
    /// `Promise -> sync`.
    fn activate_menu(&mut self, index: usize) -> Result<(), ClientError> {
        self.definitions.menus[index].window().set_flags(
            self.definitions.menus[index].window().flags() | (UiWindowFlag::HAS_FOCUS | UiWindowFlag::VISIBLE),
        );
        let on_open = self.definitions.menus[index].on_open();
        if let Some(script) = on_open {
            self.run_script(
                ScriptOwner {
                    menu: index,
                    item: None,
                },
                &script,
            )?;
        }
        let sound_loop = self.definitions.menus[index].sound_loop();
        if let Some(sound) = sound_loop {
            self.start_background(sound.path.as_deref())?;
        }
        self.close_cinematics();
        Ok(())
    }

    /// Close a menu slot (`closeMenu`).
    ///
    /// `Promise -> sync`.
    fn close_menu(&mut self, index: usize) -> Result<(), ClientError> {
        let visible = self.definitions.menus[index].window().flags() & UiWindowFlag::VISIBLE != 0;
        if visible {
            let on_close = self.definitions.menus[index].on_close();
            if let Some(script) = on_close {
                self.run_script(
                    ScriptOwner {
                        menu: index,
                        item: None,
                    },
                    &script,
                )?;
            }
        }
        self.definitions.menus[index].window().set_flags(
            self.definitions.menus[index].window().flags() & (!(UiWindowFlag::VISIBLE | UiWindowFlag::HAS_FOCUS)),
        );
        Ok(())
    }

    /// Close window and owner-draw cinematics (`closeCinematics`).
    fn close_cinematics(&mut self) {
        for menu in 0..self.active_menu_count {
            let style = self.definitions.menus[menu].window().style();
            if style == 5 {
                Self::close_window_cinematic_static(&mut self.options, &mut self.definitions.menus[menu].window());
            }
            for item in 0..self.definitions.menus[menu].item_count().max(0) as usize {
                let Ok(definition) = self.menu_item(menu, item) else {
                    continue;
                };
                let style = definition.window().style();
                let owner_draw = definition.window().owner_draw();
                let is_owner_draw = definition.behavior().kind() == "owner-draw";
                if style == 5 {
                    Self::close_window_cinematic_static(&mut self.options, &mut definition.window());
                }
                if is_owner_draw {
                    self.options.owner_draw.close_cinematic(-owner_draw);
                }
            }
        }
    }

    /// Stop a window cinematic without host errors.
    fn close_window_cinematic_static(options: &mut UiRuntimeOptions, window: &mut UiWindowDefinition) {
        if window.cinematic_handle() >= 0 {
            options.cinematics.stop(window.cinematic_handle());
            window.set_cinematic_handle(-1);
        }
    }

    /// Stop a window cinematic (`closeWindowCinematic`).
    fn close_window_cinematic(&mut self, menu: usize, item: Option<usize>) {
        let window = match item {
            Some(item) => {
                let Ok(definition) = self.menu_item(menu, item) else {
                    return;
                };
                definition.window()
            }
            None => self.definitions.menus[menu].window(),
        };
        if window.cinematic_handle() >= 0 {
            let handle = window.cinematic_handle();
            self.options.cinematics.stop(handle);
            window.set_cinematic_handle(-1);
        }
    }

    /// Start background audio (`startBackground`).
    ///
    /// `Promise -> sync`.
    fn start_background(&mut self, path: Option<&str>) -> Result<(), ClientError> {
        self.options.audio.start_background(path);
        self.opened()
    }

    /// Build a script cursor over text.
    fn script_cursor(&self, text: &str) -> Result<RuntimeScriptCursor, ClientError> {
        RuntimeScriptCursor::new(text, self.parser_line.get(), &self.definitions.memory)
    }

    /// Write a cursor line back to the shared parser.
    fn finish_cursor(&self, cursor: &RuntimeScriptCursor) {
        self.parser_line.set(cursor.line());
    }

    /// Fetch or prepare a cinematic asset (`cinematicAsset`).
    ///
    /// `Promise -> sync`.
    fn cinematic_asset(&mut self, path: &str) -> UiCinematicAsset {
        if let Some(cached) = self.cinematics.get(path) {
            return cached.clone();
        }
        let asset = self.options.resources.prepare_cinematic(path);
        self.cinematics.insert(path.to_string(), asset.clone());
        asset
    }

    /// Numeric cvar value (`cvarValue`).
    fn cvar_value(&mut self, name: &str) -> f32 {
        if let Some(compute) = self.options.cvar_value.as_mut() {
            compute(name)
        } else {
            self.options
                .cvars
                .get(name)
                .map(|found| found.numeric_value)
                .unwrap_or(0.0)
        }
    }

    /// Byte-string cvar buffer (`cvarBuffer`).
    fn cvar_buffer(&self, name: &str) -> Result<String, ClientError> {
        let value = self
            .options
            .cvars
            .get(name)
            .map(|found| found.value)
            .unwrap_or_default();
        let text = quake_string(&value, None)?;
        Ok(text.chars().take(EDIT_BUFFER_LEN - 1).collect())
    }

    /// Enable/show cvar test (`itemPassesCvar`).
    fn item_passes_cvar(&self, item: ItemState, purpose: &str) -> Result<bool, ClientError> {
        let flag = if purpose == "enable" { 1 } else { 4 };
        let definition = self.item(item)?;
        if definition.cvar_flags() & (flag | flag << 1) == 0 {
            return Ok(true);
        }
        let (script, test) = (definition.cvar_script(), definition.cvar_test());
        let (Some(script), Some(test)) = (script.as_ref(), test.as_ref()) else {
            return Ok(true);
        };
        if script.text.is_empty() || test.is_empty() {
            return Ok(true);
        }
        let current = self.cvar_buffer(test)?;
        let mut cursor = self.script_cursor(&script.text)?;
        loop {
            let value = cursor.string();
            let Some(value) = value else {
                break;
            };
            let Some(value) = value else {
                return Err(bad_ui(
                    "Item_EnableShowViaCvar dereferences NULL token after String_Alloc",
                ));
            };
            if value != ";" && equal_name(Some(&value), &current) {
                let passes = self.item(item)?.cvar_flags() & flag != 0;
                self.finish_cursor(&cursor);
                return Ok(passes);
            }
        }
        let passes = self.item(item)?.cvar_flags() & flag == 0;
        self.finish_cursor(&cursor);
        Ok(passes)
    }

    /// Run a script for an owner (`runScript`).
    ///
    /// `Promise -> sync`.
    fn run_script(&mut self, owner: ScriptOwner, script: &UiScript) -> Result<(), ClientError> {
        if script.text.is_empty() {
            return Ok(());
        }
        let mut cursor = self.script_cursor(&script.text)?;
        loop {
            let command = cursor.string();
            let Some(command) = command else {
                self.finish_cursor(&cursor);
                return Ok(());
            };
            let Some(command) = command else {
                self.finish_cursor(&cursor);
                return Err(bad_ui("Item_RunScript dereferences NULL command after String_Alloc"));
            };
            if command == ";" {
                continue;
            }
            if !self.run_shared_command(owner, &ascii_fold(&command), &mut cursor)? {
                let menu = self.script_menu(owner)?;
                let context = UiExternalScriptContext {
                    menu_name: menu.and_then(|index| self.menu(index).window().name()),
                    item_name: owner.item.and_then(|item| self.item(item).ok()?.window().name()),
                };
                self.options.external_script.run(&mut cursor, &context);
            }
        }
    }

    /// Run one shared script command (`runSharedCommand`).
    ///
    /// `Promise -> sync`. Returns whether the command was shared.
    fn run_shared_command(
        &mut self,
        owner: ScriptOwner,
        command: &str,
        cursor: &mut RuntimeScriptCursor,
    ) -> Result<bool, ClientError> {
        match command {
            "fadein" => {
                self.script_fade(owner, cursor, false)?;
                Ok(true)
            }
            "fadeout" => {
                self.script_fade(owner, cursor, true)?;
                Ok(true)
            }
            "show" => {
                self.script_show(owner, cursor, true)?;
                Ok(true)
            }
            "hide" => {
                self.script_show(owner, cursor, false)?;
                Ok(true)
            }
            "setcolor" => {
                self.script_set_color(owner.item, cursor)?;
                Ok(true)
            }
            "open" => {
                self.script_open(cursor)?;
                Ok(true)
            }
            "conditionalopen" => {
                self.script_conditional_open(cursor)?;
                Ok(true)
            }
            "close" => {
                self.script_close(cursor)?;
                Ok(true)
            }
            "setasset" => {
                let _ = cursor.string();
                Ok(true)
            }
            "setbackground" => {
                self.script_set_background(owner.item, cursor)?;
                Ok(true)
            }
            "setitemcolor" => {
                self.script_set_item_color(owner, cursor)?;
                Ok(true)
            }
            "setteamcolor" => {
                self.script_set_team_color(owner.item);
                Ok(true)
            }
            "setfocus" => {
                self.script_set_focus(owner, cursor)?;
                Ok(true)
            }
            "setplayermodel" => {
                self.script_set_cvar(cursor, Some("team_model"));
                Ok(true)
            }
            "setplayerhead" => {
                self.script_set_cvar(cursor, Some("team_headmodel"));
                Ok(true)
            }
            "transition" => {
                self.script_transition(owner, cursor)?;
                Ok(true)
            }
            "setcvar" => {
                self.script_set_cvar(cursor, None);
                Ok(true)
            }
            "exec" => {
                self.script_exec(cursor)?;
                Ok(true)
            }
            "play" => {
                self.script_play(cursor);
                Ok(true)
            }
            "playlooped" => {
                self.script_play_looped(cursor)?;
                Ok(true)
            }
            "orbit" => {
                self.script_orbit(owner, cursor)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Script owner menu (`scriptMenu`).
    fn script_menu(&self, owner: ScriptOwner) -> Result<Option<usize>, ClientError> {
        match owner.item {
            None => Ok(Some(owner.menu)),
            Some(item) => self.parent_menu(item),
        }
    }

    /// Show or hide grouped items (`scriptShow`).
    fn script_show(
        &mut self,
        owner: ScriptOwner,
        cursor: &mut RuntimeScriptCursor,
        visible: bool,
    ) -> Result<(), ClientError> {
        let name = cursor.string();
        let Some(name) = name else {
            return Ok(());
        };
        let menu = self.script_menu(owner)?;
        let Some(menu) = menu else {
            return Err(bad_ui("Menu_ItemsMatchingGroup dereferences a NULL parent menu"));
        };
        for item in self.matching_indices(menu, name.as_deref()) {
            let target = ItemState {
                menu_index: menu,
                item_index: item,
            };
            if visible {
                self.menu_item(menu, item)?
                    .window()
                    .set_flags(self.menu_item(menu, item)?.window().flags() | UiWindowFlag::VISIBLE);
            } else {
                self.menu_item(menu, item)?
                    .window()
                    .set_flags(self.menu_item(menu, item)?.window().flags() & !UiWindowFlag::VISIBLE);
                self.close_cinematics_for(target);
            }
        }
        Ok(())
    }

    /// Stop an item cinematic.
    fn close_cinematics_for(&mut self, item: ItemState) {
        self.close_window_cinematic(item.menu_index, Some(item.item_index));
    }

    /// Fade grouped items (`scriptFade`).
    fn script_fade(
        &mut self,
        owner: ScriptOwner,
        cursor: &mut RuntimeScriptCursor,
        fade_out: bool,
    ) -> Result<(), ClientError> {
        let name = cursor.string();
        let Some(name) = name else {
            return Ok(());
        };
        let menu = self.script_menu(owner)?;
        let Some(menu) = menu else {
            return Err(bad_ui("Menu_ItemsMatchingGroup dereferences a NULL parent menu"));
        };
        for item in self.matching_indices(menu, name.as_deref()) {
            let window = self.menu_item(menu, item)?.window();
            let mut flags = window.flags();
            if fade_out {
                flags |= UiWindowFlag::FADING_OUT | UiWindowFlag::VISIBLE;
                flags &= !UiWindowFlag::FADING_IN;
            } else {
                flags |= UiWindowFlag::VISIBLE | UiWindowFlag::FADING_IN;
                flags &= !UiWindowFlag::FADING_OUT;
            }
            window.set_flags(flags);
        }
        Ok(())
    }

    /// Set an item color (`scriptSetColor`).
    fn script_set_color(
        &mut self,
        item: Option<ItemState>,
        cursor: &mut RuntimeScriptCursor,
    ) -> Result<(), ClientError> {
        let target = cursor.string();
        let Some(Some(target)) = target else {
            return Ok(());
        };
        let name = ascii_fold(&target);
        if name != "backcolor" && name != "forecolor" && name != "bordercolor" {
            return Ok(());
        }
        if let Some(item) = item {
            let window = self.item(item)?.window();
            let mut flags = window.flags();
            if name == "backcolor" {
                flags |= UiWindowFlag::BACK_COLOR_SET;
            } else if name == "forecolor" {
                flags |= UiWindowFlag::FORE_COLOR_SET;
            }
            window.set_flags(flags);
        }
        for component in 0..4 {
            let value = cursor.raw_string();
            let Some(value) = value else {
                return Ok(());
            };
            let parsed = game_atof(&value)?;
            if let Some(item) = item {
                let window = self.item(item)?.window();
                let color = match name.as_str() {
                    "backcolor" => window.back_color(),
                    "forecolor" => window.fore_color(),
                    _ => window.border_color(),
                };
                match component {
                    0 => color.set_x(parsed),
                    1 => color.set_y(parsed),
                    2 => color.set_z(parsed),
                    _ => color.set_w(parsed),
                }
            }
        }
        Ok(())
    }

    /// Open a menu by script (`scriptOpen`).
    ///
    /// `Promise -> sync`.
    fn script_open(&mut self, cursor: &mut RuntimeScriptCursor) -> Result<(), ClientError> {
        let name = cursor.string();
        let Some(name) = name else {
            return Ok(());
        };
        match name {
            Some(name) => {
                self.activate(UiMenuSelector::Name(name))?;
            }
            None => {
                self.activate(UiMenuSelector::Null)?;
            }
        }
        Ok(())
    }

    /// Conditionally open a menu (`scriptConditionalOpen`).
    ///
    /// `Promise -> sync`.
    fn script_conditional_open(&mut self, cursor: &mut RuntimeScriptCursor) -> Result<(), ClientError> {
        let cvar = cursor.string();
        let Some(cvar) = cvar else {
            return Ok(());
        };
        let first = cursor.string();
        let Some(first) = first else {
            return Ok(());
        };
        let second = cursor.string();
        let Some(second) = second else {
            return Ok(());
        };
        let Some(cvar) = cvar else {
            return Err(bad_ui("Cvar_VariableValue hashes NULL in Cvar_FindVar"));
        };
        let value = self.cvar_value(&cvar);
        let selected = if value == 0.0 { second } else { first };
        match selected {
            Some(name) => {
                self.activate(UiMenuSelector::Name(name))?;
            }
            None => {
                self.activate(UiMenuSelector::Null)?;
            }
        }
        Ok(())
    }

    /// Close a menu by script (`scriptClose`).
    ///
    /// `Promise -> sync`.
    fn script_close(&mut self, cursor: &mut RuntimeScriptCursor) -> Result<(), ClientError> {
        let name = cursor.string();
        if let Some(Some(name)) = name {
            self.close(&name)?;
        }
        Ok(())
    }

    /// Set an item background (`scriptSetBackground`).
    ///
    /// `Promise -> sync`.
    fn script_set_background(
        &mut self,
        item: Option<ItemState>,
        cursor: &mut RuntimeScriptCursor,
    ) -> Result<(), ClientError> {
        let path = cursor.string();
        let Some(path) = path else {
            return Ok(());
        };
        let background = self.options.resources.register_picture(path.as_deref());
        let key = resource_key(path.as_deref());
        if let Some(item) = item {
            let handle = if self.options.resources.handle_kind() == UiHandleKind::Source {
                Some(self.options.resources.picture_handle(background)?)
            } else {
                None
            };
            self.menu_item(item.menu_index, item.item_index)?
                .window()
                .set_background(UiShaderReference { path }, handle);
        }
        self.pictures.insert(key, background);
        Ok(())
    }

    /// Set grouped item colors (`scriptSetItemColor`).
    fn script_set_item_color(
        &mut self,
        owner: ScriptOwner,
        cursor: &mut RuntimeScriptCursor,
    ) -> Result<(), ClientError> {
        let item_name = cursor.string();
        let Some(item_name) = item_name else {
            return Ok(());
        };
        let target = cursor.string();
        let Some(target) = target else {
            return Ok(());
        };
        let menu = self.script_menu(owner)?;
        let Some(menu) = menu else {
            return Err(bad_ui("Menu_ItemsMatchingGroup dereferences a NULL parent menu"));
        };
        let indices = self.matching_indices(menu, item_name.as_deref());
        let x = cursor.raw_string();
        let Some(x) = x else {
            return Ok(());
        };
        let y = cursor.raw_string();
        let Some(y) = y else {
            return Ok(());
        };
        let z = cursor.raw_string();
        let Some(z) = z else {
            return Ok(());
        };
        let w = cursor.raw_string();
        let Some(w) = w else {
            return Ok(());
        };
        let color = Vec4 {
            x: game_atof(&x)?,
            y: game_atof(&y)?,
            z: game_atof(&z)?,
            w: game_atof(&w)?,
        };
        let Some(target) = target else {
            return Ok(());
        };
        let name = ascii_fold(&target);
        for item in indices {
            if name == "backcolor" {
                self.menu_item(menu, item)?.window().set_back_color(&color);
            } else if name == "forecolor" {
                self.menu_item(menu, item)?
                    .window()
                    .set_flags(self.menu_item(menu, item)?.window().flags() | UiWindowFlag::FORE_COLOR_SET);
                self.menu_item(menu, item)?.window().set_fore_color(&color);
            } else if name == "bordercolor" {
                self.menu_item(menu, item)?.window().set_border_color(&color);
            }
        }
        Ok(())
    }

    /// Set an item's team color (`scriptSetTeamColor`).
    fn script_set_team_color(&mut self, item: Option<ItemState>) {
        let color = (self.options.get_team_color)();
        if let Some(item) = item {
            let Ok(definition) = self.item(item) else {
                return;
            };
            definition.window().set_back_color(&color);
        }
    }

    /// Clear item focus (`clearFocus`).
    ///
    /// `Promise -> sync`.
    fn clear_focus(&mut self, menu: Option<usize>) -> Result<Option<ItemState>, ClientError> {
        let Some(menu) = menu else {
            return Ok(None);
        };
        let mut previous = None;
        for item in 0..self.definitions.menus[menu].item_count().max(0) as usize {
            if self.menu_item(menu, item)?.window().flags() & UiWindowFlag::HAS_FOCUS != 0 {
                previous = Some(ItemState {
                    menu_index: menu,
                    item_index: item,
                });
            }
            self.menu_item(menu, item)?
                .window()
                .set_flags(self.menu_item(menu, item)?.window().flags() & !UiWindowFlag::HAS_FOCUS);
            let leave = self.menu_item(menu, item)?.leave_focus();
            if let Some(script) = leave {
                self.run_script(
                    ScriptOwner {
                        menu,
                        item: Some(ItemState {
                            menu_index: menu,
                            item_index: item,
                        }),
                    },
                    &script,
                )?;
            }
        }
        Ok(previous)
    }

    /// Focus an item by script (`scriptSetFocus`).
    ///
    /// `Promise -> sync`.
    fn script_set_focus(&mut self, owner: ScriptOwner, cursor: &mut RuntimeScriptCursor) -> Result<(), ClientError> {
        let name = cursor.string();
        let Some(Some(name)) = name else {
            return Ok(());
        };
        let menu = self.script_menu(owner)?;
        let Some(menu) = menu else {
            return Ok(());
        };
        let definition = &self.definitions.menus[menu];
        let mut found = None;
        for slot in 0..definition.item_count().max(0) as usize {
            let Ok(candidate) = definition.item_view(slot) else {
                continue;
            };
            if equal_name(candidate.window().name().as_deref(), &name) {
                found = Some(slot);
                break;
            }
        }
        let Some(found) = found else {
            return Ok(());
        };
        let flags = self.menu_item(menu, found)?.window().flags();
        if flags & UiWindowFlag::DECORATION != 0 || flags & UiWindowFlag::HAS_FOCUS != 0 {
            return Ok(());
        }
        let menu_again = self.script_menu(owner)?;
        self.clear_focus(menu_again)?;
        self.menu_item(menu, found)?
            .window()
            .set_flags(self.menu_item(menu, found)?.window().flags() | UiWindowFlag::HAS_FOCUS);
        let on_focus = self.menu_item(menu, found)?.on_focus();
        if let Some(script) = on_focus {
            self.run_script(
                ScriptOwner {
                    menu,
                    item: Some(ItemState {
                        menu_index: menu,
                        item_index: found,
                    }),
                },
                &script,
            )?;
        }
        let global = self.definitions.assets.item_focus_sound.clone();
        if let Some(path) = global.as_ref().and_then(|found| found.path.as_deref()) {
            let sound = self.sound(Some(path));
            if sound.is_some() {
                self.options.audio.play_local(sound.map(UiLocalSound::Pcm));
            }
        }
        Ok(())
    }

    /// Set a cvar by script (`scriptSetCvar`).
    fn script_set_cvar(&mut self, cursor: &mut RuntimeScriptCursor, fixed_name: Option<&str>) {
        let name = match fixed_name {
            Some(name) => Some(Some(name.to_string())),
            None => cursor.string(),
        };
        let Some(name) = name else {
            return;
        };
        let value = cursor.string();
        let Some(value) = value else {
            return;
        };
        self.set_cvar(name.as_deref(), value.as_deref(), true);
    }

    /// Execute console text by script (`scriptExec`).
    fn script_exec(&mut self, cursor: &mut RuntimeScriptCursor) -> Result<(), ClientError> {
        let value = cursor.string();
        if let Some(value) = value {
            let text = game_format("%s ; ", &[GameFormatArg::Text(value)])?;
            self.execute_text(&text)?;
        }
        Ok(())
    }

    /// Play a sound by script (`scriptPlay`).
    ///
    /// `Promise -> sync`.
    fn script_play(&mut self, cursor: &mut RuntimeScriptCursor) {
        let path = cursor.string();
        if let Some(path) = path {
            let sound = self.options.resources.register_sound(path.as_deref());
            self.sounds.insert(resource_key(path.as_deref()), sound.clone());
            self.options.audio.play_local(sound.map(UiLocalSound::Pcm));
        }
    }

    /// Loop background audio by script (`scriptPlayLooped`).
    ///
    /// `Promise -> sync`.
    fn script_play_looped(&mut self, cursor: &mut RuntimeScriptCursor) -> Result<(), ClientError> {
        let path = cursor.string();
        let Some(path) = path else {
            return Ok(());
        };
        self.options.audio.stop_background();
        self.start_background(path.as_deref())
    }

    /// Transition grouped items (`scriptTransition`).
    fn script_transition(&mut self, owner: ScriptOwner, cursor: &mut RuntimeScriptCursor) -> Result<(), ClientError> {
        let name = cursor.string();
        let Some(name) = name else {
            return Ok(());
        };
        let from = parsed_rect(cursor)?;
        let Some(from) = from else {
            return Ok(());
        };
        let to = parsed_rect(cursor)?;
        let Some(to) = to else {
            return Ok(());
        };
        let time_text = cursor.raw_string();
        let Some(time_text) = time_text else {
            return Ok(());
        };
        let amount_text = cursor.raw_string();
        let Some(amount_text) = amount_text else {
            return Ok(());
        };
        let amount = game_atof(&amount_text)?;
        let menu = self.script_menu(owner)?;
        let Some(menu) = menu else {
            return Err(bad_ui("Menu_ItemsMatchingGroup dereferences a NULL parent menu"));
        };
        for item in self.matching_indices(menu, name.as_deref()) {
            let target = ItemState {
                menu_index: menu,
                item_index: item,
            };
            {
                let definition = self.menu_item(menu, item)?;
                definition
                    .window()
                    .set_flags(definition.window().flags() | UiWindowFlag::IN_TRANSITION | UiWindowFlag::VISIBLE);
                definition.window().set_offset_time(game_atoi(&time_text)?);
                definition.window().set_client_rect(&from);
                definition.window().set_rect_effects(&to);
                definition.window().set_rect_effects2(&UiRect {
                    x: transition_step(from.x, to.x, amount),
                    y: transition_step(from.y, to.y, amount),
                    width: transition_step(from.width, to.width, amount),
                    height: transition_step(from.height, to.height, amount),
                });
            }
            self.update_item_position(target)?;
        }
        Ok(())
    }

    /// Orbit grouped items (`scriptOrbit`).
    fn script_orbit(&mut self, owner: ScriptOwner, cursor: &mut RuntimeScriptCursor) -> Result<(), ClientError> {
        let name = cursor.string();
        let Some(name) = name else {
            return Ok(());
        };
        let x = cursor.raw_string();
        let Some(x) = x else {
            return Ok(());
        };
        let y = cursor.raw_string();
        let Some(y) = y else {
            return Ok(());
        };
        let cx = cursor.raw_string();
        let Some(cx) = cx else {
            return Ok(());
        };
        let cy = cursor.raw_string();
        let Some(cy) = cy else {
            return Ok(());
        };
        let time = cursor.raw_string();
        let Some(time) = time else {
            return Ok(());
        };
        let menu = self.script_menu(owner)?;
        let Some(menu) = menu else {
            return Err(bad_ui("Menu_ItemsMatchingGroup dereferences a NULL parent menu"));
        };
        for item in self.matching_indices(menu, name.as_deref()) {
            let target = ItemState {
                menu_index: menu,
                item_index: item,
            };
            {
                let definition = self.menu_item(menu, item)?;
                definition
                    .window()
                    .set_flags(definition.window().flags() | UiWindowFlag::ORBITING | UiWindowFlag::VISIBLE);
                definition.window().set_offset_time(game_atoi(&time)?);
                definition.window().rect_effects().set_x(game_atof(&cx)?);
                definition.window().rect_effects().set_y(game_atof(&cy)?);
                definition.window().client_rect().set_x(game_atof(&x)?);
                definition.window().client_rect().set_y(game_atof(&y)?);
            }
            self.update_item_position(target)?;
        }
        Ok(())
    }

    /// Reposition an item under its parent (`updateItemPosition`).
    fn update_item_position(&mut self, item: ItemState) -> Result<(), ClientError> {
        let parent = self.item(item)?.parent();
        let Some(parent) = parent else {
            return Ok(());
        };
        let Some(slot) = self.definitions.menus.iter().position(|menu| menu.same_record(&parent)) else {
            return Ok(());
        };
        let (mut x, mut y, border, border_size) = {
            let menu = &self.definitions.menus[slot];
            (
                menu.window().rect().x(),
                menu.window().rect().y(),
                menu.window().border(),
                menu.window().border_size(),
            )
        };
        if border != 0 {
            x = f(x + border_size);
            y = f(y + border_size);
        }
        let definition = self.definitions.menus[item.menu_index]
            .item_at(item.item_index)
            .ok_or_else(|| bad_ui("UI menu item index is out of range"))?;
        set_item_screen_coords(&definition, x, y);
        Ok(())
    }

    /// Text hit rectangle (`correctedTextRect`).
    fn corrected_text_rect(&self, item: ItemState) -> Result<UiRect, ClientError> {
        let text = self.item(item)?.text_rect();
        if text.width() == 0.0 {
            Ok(UiRect {
                x: text.x(),
                y: text.y(),
                width: text.width(),
                height: text.height(),
            })
        } else {
            Ok(UiRect {
                x: text.x(),
                y: f(text.y() - text.height()),
                width: text.width(),
                height: text.height(),
            })
        }
    }

    /// Route pointer motion within a menu (`mouseMoveMenu`).
    ///
    /// `Promise -> sync`.
    fn mouse_move_menu(&mut self, menu: usize, x: f32, y: f32) -> Result<(), ClientError> {
        if self.menu(menu).window().flags() & (UiWindowFlag::VISIBLE | UiWindowFlag::FORCED) == 0 {
            return Ok(());
        }
        if !matches!(self.capture, CaptureState::Idle) || self.waiting_for_key || self.editing_item.is_some() {
            return Ok(());
        }
        let mut focus_set = false;
        for pass in 0..2 {
            for item in 0..self.menu(menu).item_count().max(0) as usize {
                let target = ItemState {
                    menu_index: menu,
                    item_index: item,
                };
                let flags = self.item(target)?.window().flags();
                if flags & (UiWindowFlag::VISIBLE | UiWindowFlag::FORCED) == 0 {
                    continue;
                }
                if !self.item_passes_cvar(target, "enable")? || !self.item_passes_cvar(target, "show")? {
                    continue;
                }
                if rect_contains(&self.item(target)?.window().rect().snapshot(), x, y) {
                    if pass != 1 {
                        continue;
                    }
                    let is_text = self.item(target)?.behavior().kind() == "text";
                    let has_text = self.item(target)?.text().is_some();
                    if is_text && has_text && !rect_contains(&self.corrected_text_rect(target)?, x, y) {
                        continue;
                    }
                    let flags = self.item(target)?.window().flags();
                    if flags & UiWindowFlag::VISIBLE != 0 && flags & UiWindowFlag::FADING_OUT == 0 {
                        self.mouse_enter(menu, target, x, y)?;
                        if !focus_set {
                            focus_set = self.set_item_focus(menu, target, x, y)?;
                        }
                    }
                } else if self.item(target)?.window().flags() & UiWindowFlag::MOUSE_OVER != 0 {
                    self.mouse_leave(menu, target)?;
                    self.menu_item(menu, item)?
                        .window()
                        .set_flags(self.menu_item(menu, item)?.window().flags() & !UiWindowFlag::MOUSE_OVER);
                }
            }
        }
        Ok(())
    }

    /// Run mouse-enter scripts (`mouseEnter`).
    ///
    /// `Promise -> sync`.
    fn mouse_enter(&mut self, menu: usize, item: ItemState, x: f32, y: f32) -> Result<(), ClientError> {
        if !self.item_passes_cvar(item, "enable")? || !self.item_passes_cvar(item, "show")? {
            return Ok(());
        }
        let text = self.item(item)?.text_rect();
        let over_text = rect_contains(
            &UiRect {
                y: f(text.y() - text.height()),
                ..text.snapshot()
            },
            x,
            y,
        );
        if over_text {
            if self.item(item)?.window().flags() & UiWindowFlag::MOUSE_OVER_TEXT == 0 {
                let script = self.item(item)?.mouse_enter_text();
                if let Some(script) = script {
                    self.run_script(ScriptOwner { menu, item: Some(item) }, &script)?;
                }
                self.menu_item(menu, item.item_index)?
                    .window()
                    .set_flags(self.menu_item(menu, item.item_index)?.window().flags() | UiWindowFlag::MOUSE_OVER_TEXT);
            }
            if self.item(item)?.window().flags() & UiWindowFlag::MOUSE_OVER == 0 {
                let script = self.item(item)?.mouse_enter();
                if let Some(script) = script {
                    self.run_script(ScriptOwner { menu, item: Some(item) }, &script)?;
                }
                self.menu_item(menu, item.item_index)?
                    .window()
                    .set_flags(self.menu_item(menu, item.item_index)?.window().flags() | UiWindowFlag::MOUSE_OVER);
            }
        } else {
            if self.item(item)?.window().flags() & UiWindowFlag::MOUSE_OVER_TEXT != 0 {
                let script = self.item(item)?.mouse_exit_text();
                if let Some(script) = script {
                    self.run_script(ScriptOwner { menu, item: Some(item) }, &script)?;
                }
                self.menu_item(menu, item.item_index)?.window().set_flags(
                    self.menu_item(menu, item.item_index)?.window().flags() & !UiWindowFlag::MOUSE_OVER_TEXT,
                );
            }
            if self.item(item)?.window().flags() & UiWindowFlag::MOUSE_OVER == 0 {
                let script = self.item(item)?.mouse_enter();
                if let Some(script) = script {
                    self.run_script(ScriptOwner { menu, item: Some(item) }, &script)?;
                }
                self.menu_item(menu, item.item_index)?
                    .window()
                    .set_flags(self.menu_item(menu, item.item_index)?.window().flags() | UiWindowFlag::MOUSE_OVER);
            }
            if self.item(item)?.item_type() == UiItemTypeCode::ListBox as i32 {
                self.list_mouse_enter(item, x, y)?;
            }
        }
        Ok(())
    }

    /// Run mouse-leave scripts (`mouseLeave`).
    ///
    /// `Promise -> sync`.
    fn mouse_leave(&mut self, menu: usize, item: ItemState) -> Result<(), ClientError> {
        if self.item(item)?.window().flags() & UiWindowFlag::MOUSE_OVER_TEXT != 0 {
            let script = self.item(item)?.mouse_exit_text();
            if let Some(script) = script {
                self.run_script(ScriptOwner { menu, item: Some(item) }, &script)?;
            }
            self.menu_item(menu, item.item_index)?
                .window()
                .set_flags(self.menu_item(menu, item.item_index)?.window().flags() & !UiWindowFlag::MOUSE_OVER_TEXT);
        }
        let script = self.item(item)?.mouse_exit();
        if let Some(script) = script {
            self.run_script(ScriptOwner { menu, item: Some(item) }, &script)?;
        }
        self.menu_item(menu, item.item_index)?.window().set_flags(
            self.menu_item(menu, item.item_index)?.window().flags()
                & !(UiWindowFlag::LIST_RIGHT_ARROW | UiWindowFlag::LIST_LEFT_ARROW),
        );
        Ok(())
    }

    /// Focus an item (`setItemFocus`).
    ///
    /// `Promise -> sync`.
    fn set_item_focus(&mut self, menu: usize, item: ItemState, x: f32, y: f32) -> Result<bool, ClientError> {
        let flags = self.item(item)?.window().flags();
        if flags & (UiWindowFlag::DECORATION | UiWindowFlag::HAS_FOCUS) != 0 || flags & UiWindowFlag::VISIBLE == 0 {
            return Ok(false);
        }
        let parent = self.parent_menu(item)?;
        if !self.item_passes_cvar(item, "enable")? || !self.item_passes_cvar(item, "show")? {
            return Ok(false);
        }
        let old_focus = self.clear_focus(parent)?;
        let mut play_sound = false;
        if self.item(item)?.behavior().kind() == "text" {
            if rect_contains(&self.corrected_text_rect(item)?, x, y) {
                let window = self.menu_item(item.menu_index, item.item_index)?.window();
                window.set_flags(window.flags() | UiWindowFlag::HAS_FOCUS);
                play_sound = true;
            } else if let Some(old) = old_focus {
                let window = self.menu_item(old.menu_index, old.item_index)?.window();
                window.set_flags(window.flags() | UiWindowFlag::HAS_FOCUS);
                let script = self.item(old)?.on_focus();
                if let Some(script) = script {
                    self.run_script(ScriptOwner { menu, item: Some(old) }, &script)?;
                }
            }
        } else {
            let window = self.menu_item(item.menu_index, item.item_index)?.window();
            window.set_flags(window.flags() | UiWindowFlag::HAS_FOCUS);
            let script = self.item(item)?.on_focus();
            if let Some(script) = script {
                self.run_script(ScriptOwner { menu, item: Some(item) }, &script)?;
            }
            play_sound = true;
        }
        if play_sound {
            let handle = self.item(item)?.focus_sound_handle();
            match handle {
                Some(handle) if handle != 0 => {
                    self.options.audio.play_local(Some(UiLocalSound::Handle(handle)));
                }
                _ => {
                    let item_path = match handle {
                        None => self.item(item)?.focus_sound(),
                        Some(_) => None,
                    };
                    let item_sound = item_path
                        .as_ref()
                        .and_then(|found| found.path.as_deref())
                        .and_then(|path| self.sound(Some(path)));
                    let global_path = self.definitions.assets.item_focus_sound.clone();
                    let global_sound = global_path
                        .as_ref()
                        .and_then(|found| found.path.as_deref())
                        .and_then(|path| self.sound(Some(path)));
                    let selected = item_sound.or(global_sound);
                    self.options.audio.play_local(selected.map(UiLocalSound::Pcm));
                }
            }
        }
        let Some(parent) = parent else {
            return Err(bad_ui("Item_SetFocus dereferences a NULL parent menu at itemCount"));
        };
        if parent == item.menu_index {
            self.definitions.menus[parent].set_cursor_item(item.item_index as i32);
        }
        Ok(true)
    }

    /// Focus a relative item (`focusRelative`).
    ///
    /// `Promise -> sync`.
    fn focus_relative(&mut self, menu: usize, direction: i32) -> Result<Option<ItemState>, ClientError> {
        let original = self.definitions.menus[menu].cursor_item();
        let mut wrapped = false;
        if direction < 0 && self.definitions.menus[menu].cursor_item() < 0 {
            self.definitions.menus[menu].set_cursor_item(self.definitions.menus[menu].item_count() - 1);
            wrapped = true;
        } else if direction > 0 && self.definitions.menus[menu].cursor_item() == -1 {
            self.definitions.menus[menu].set_cursor_item(0);
            wrapped = true;
        }
        loop {
            let cursor = self.definitions.menus[menu].cursor_item();
            let count = self.definitions.menus[menu].item_count();
            if !(if direction < 0 { cursor > -1 } else { cursor < count }) {
                break;
            }
            self.definitions.menus[menu].set_cursor_item(self.definitions.menus[menu].cursor_item() + (direction));
            let cursor = self.definitions.menus[menu].cursor_item();
            if (if direction < 0 { cursor < 0 } else { cursor >= count }) && !wrapped {
                wrapped = true;
                self.definitions.menus[menu].set_cursor_item(if direction < 0 { count - 1 } else { 0 });
            }
            let cursor = self.definitions.menus[menu].cursor_item();
            let target = if cursor >= 0 && (cursor as usize) < self.definitions.menus[menu].item_count().max(0) as usize
            {
                Some(ItemState {
                    menu_index: menu,
                    item_index: cursor as usize,
                })
            } else {
                None
            };
            if let Some(target) = target {
                let (cursor_x, cursor_y) = (self.display_cursor_x, self.display_cursor_y);
                if self.set_item_focus(menu, target, cursor_x, cursor_y)? {
                    let rect = self.item(target)?.window().rect();
                    self.mouse_move_menu(menu, f(rect.x() + 1.0), f(rect.y() + 1.0))?;
                    let cursor = self.definitions.menus[menu].cursor_item();
                    if cursor >= 0 && (cursor as usize) < self.definitions.menus[menu].item_count().max(0) as usize {
                        return Ok(Some(ItemState {
                            menu_index: menu,
                            item_index: cursor as usize,
                        }));
                    }
                    return Ok(None);
                }
            }
        }
        self.definitions.menus[menu].set_cursor_item(original);
        Ok(None)
    }

    /// Handle a menu key (`handleMenuKey`).
    ///
    /// `Promise -> sync`.
    fn handle_menu_key(&mut self, menu: usize, key: i32, down: bool) -> Result<bool, ClientError> {
        if self.waiting_for_key && down {
            return match self.binding_item {
                None => Ok(true),
                Some(item) => self.handle_bind_key(item, key, down),
            };
        }
        if let Some(editing) = self.editing_item {
            if down {
                if !self.handle_text_key(editing, key)? {
                    self.editing_item = None;
                    return Ok(true);
                }
                if is_mouse_key(key) {
                    self.editing_item = None;
                    let (x, y) = (self.display_cursor_x, self.display_cursor_y);
                    self.pointer_move(x, y)?;
                } else if key == KeyCode::Tab as i32 || key == KeyCode::Up as i32 || key == KeyCode::Down as i32 {
                    return Ok(true);
                }
            }
        }
        if down
            && self.menu(menu).window().flags() & UiWindowFlag::POPUP == 0
            && !rect_contains(
                &self.menu(menu).window().rect().snapshot(),
                self.display_cursor_x,
                self.display_cursor_y,
            )
            && is_mouse_key(key)
        {
            self.handle_out_of_bounds(menu, key, down)?;
            return Ok(true);
        }
        let mut item = None;
        for index in 0..self.menu(menu).item_count().max(0) as usize {
            if self.menu_item(menu, index)?.window().flags() & UiWindowFlag::HAS_FOCUS != 0 {
                item = Some(ItemState {
                    menu_index: menu,
                    item_index: index,
                });
            }
        }
        if let Some(item) = item {
            if self.handle_item_key(item, key, down)? {
                let action = self.item(item)?.action();
                if let Some(script) = action {
                    self.run_script(ScriptOwner { menu, item: Some(item) }, &script)?;
                }
                return Ok(true);
            }
        }
        if !down {
            return Ok(false);
        }
        if key == KeyCode::F11 as i32 {
            if self.cvar_value("developer") != 0.0 {
                self.debug = !self.debug;
            }
            return Ok(true);
        }
        if key == KeyCode::F12 as i32 {
            if self.cvar_value("developer") != 0.0 {
                self.execute_text("screenshot\n")?;
            }
            return Ok(true);
        }
        if key == KeyCode::Up as i32 || key == KeyCode::KeypadUp as i32 {
            self.focus_relative(menu, -1)?;
            return Ok(true);
        }
        if key == KeyCode::Down as i32 || key == KeyCode::KeypadDown as i32 || key == KeyCode::Tab as i32 {
            self.focus_relative(menu, 1)?;
            return Ok(true);
        }
        if key == KeyCode::Escape as i32 {
            if !self.waiting_for_key {
                let on_escape = self.menu(menu).on_escape();
                if let Some(script) = on_escape {
                    self.run_script(ScriptOwner { menu, item: None }, &script)?;
                }
            }
            return Ok(true);
        }
        if key == KeyCode::Mouse1 as i32 || key == KeyCode::Mouse2 as i32 {
            if let Some(item) = item {
                let kind = self.item(item)?.behavior().kind();
                if kind == "text" {
                    if rect_contains(
                        &self.corrected_text_rect(item)?,
                        self.display_cursor_x,
                        self.display_cursor_y,
                    ) {
                        let action = self.item(item)?.action();
                        if let Some(script) = action {
                            self.run_script(ScriptOwner { menu, item: Some(item) }, &script)?;
                        }
                    }
                } else if kind == "edit-field" || kind == "numeric-field" {
                    if rect_contains(
                        &self.item(item)?.window().rect().snapshot(),
                        self.display_cursor_x,
                        self.display_cursor_y,
                    ) {
                        self.begin_editing(item)?;
                    }
                } else if rect_contains(
                    &self.item(item)?.window().rect().snapshot(),
                    self.display_cursor_x,
                    self.display_cursor_y,
                ) {
                    let action = self.item(item)?.action();
                    if let Some(script) = action {
                        self.run_script(ScriptOwner { menu, item: Some(item) }, &script)?;
                    }
                }
                return Ok(true);
            }
        }
        if key == KeyCode::Enter as i32 || key == KeyCode::KeypadEnter as i32 {
            if let Some(item) = item {
                let kind = self.item(item)?.behavior().kind();
                if kind == "edit-field" || kind == "numeric-field" {
                    self.begin_editing(item)?;
                } else {
                    let action = self.item(item)?.action();
                    if let Some(script) = action {
                        self.run_script(ScriptOwner { menu, item: Some(item) }, &script)?;
                    }
                }
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Begin editing an item (`beginEditing`).
    fn begin_editing(&mut self, item: ItemState) -> Result<(), ClientError> {
        self.menu_item(item.menu_index, item.item_index)?.set_cursor_position(0);
        self.editing_item = Some(item);
        self.binding_host("edit overstrike")?.set_overstrike(true);
        Ok(())
    }

    /// Handle an out-of-bounds click (`handleOutOfBounds`).
    ///
    /// `Promise -> sync`.
    fn handle_out_of_bounds(&mut self, menu: usize, key: i32, down: bool) -> Result<(), ClientError> {
        if down && self.menu(menu).window().flags() & UiWindowFlag::OUT_OF_BOUNDS_CLICK != 0 {
            self.close_menu(menu)?;
        }
        for candidate in 0..self.active_menu_count {
            if !self.menu_over_active_item(candidate, self.display_cursor_x, self.display_cursor_y)? {
                continue;
            }
            self.close_menu(menu)?;
            self.activate_menu(candidate)?;
            let (x, y) = (self.display_cursor_x, self.display_cursor_y);
            self.mouse_move_menu(candidate, x, y)?;
            self.handle_menu_key(candidate, key, down)?;
        }
        let any_visible = self.definitions.menus[..self.active_menu_count]
            .iter()
            .any(|menu| menu.window().flags() & (UiWindowFlag::FORCED | UiWindowFlag::VISIBLE) != 0);
        if !any_visible {
            if let UiRuntimeContext::Ui { pause, .. } = &mut self.options.context {
                pause(false);
            }
            self.opened()?;
        }
        self.close_cinematics();
        Ok(())
    }

    /// Whether a point hits an active item (`menuOverActiveItem`).
    fn menu_over_active_item(&self, menu: usize, x: f32, y: f32) -> Result<bool, ClientError> {
        if self.menu(menu).window().flags() & (UiWindowFlag::VISIBLE | UiWindowFlag::FORCED) == 0
            || !rect_contains(&self.menu(menu).window().rect().snapshot(), x, y)
        {
            return Ok(false);
        }
        for item in 0..self.menu(menu).item_count().max(0) as usize {
            let target = ItemState {
                menu_index: menu,
                item_index: item,
            };
            let flags = self.item(target)?.window().flags();
            if flags & (UiWindowFlag::VISIBLE | UiWindowFlag::FORCED) == 0 || flags & UiWindowFlag::DECORATION != 0 {
                continue;
            }
            if !rect_contains(&self.item(target)?.window().rect().snapshot(), x, y) {
                continue;
            }
            let is_text = self.item(target)?.behavior().kind() == "text";
            let has_text = self.item(target)?.text().is_some();
            if !is_text || !has_text || rect_contains(&self.corrected_text_rect(target)?, x, y) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Handle an item key (`handleItemKey`).
    ///
    /// `Promise -> sync`.
    fn handle_item_key(&mut self, item: ItemState, key: i32, down: bool) -> Result<bool, ClientError> {
        if !matches!(self.capture, CaptureState::Idle) {
            self.capture = CaptureState::Idle;
        } else if down && is_mouse_key(key) {
            self.start_capture(item, key)?;
        }
        if !down {
            return Ok(false);
        }
        match self.item(item)?.behavior().kind() {
            "list-box" => self.handle_list_key(item, key, false),
            "yes-no" => self.handle_yes_no_key(item, key),
            "multi" => self.handle_multi_key(item, key),
            "owner-draw" => {
                let (owner_draw, flags, special) = {
                    let definition = self.item(item)?;
                    (
                        definition.window().owner_draw(),
                        definition.window().owner_draw_flags(),
                        definition.special(),
                    )
                };
                let result = self.options.owner_draw.handle_key(owner_draw, flags, special, key);
                self.menu_item(item.menu_index, item.item_index)?
                    .set_special(result.special);
                Ok(result.handled)
            }
            "bind" => self.handle_bind_key(item, key, down),
            "slider" => self.handle_slider_key(item, key),
            _ => Ok(false),
        }
    }

    /// Handle a yes/no key (`handleYesNoKey`).
    fn handle_yes_no_key(&mut self, item: ItemState, key: i32) -> Result<bool, ClientError> {
        let cvar = self.item(item)?.cvar();
        let Some(cvar) = cvar else {
            return Ok(false);
        };
        if self.item(item)?.window().flags() & UiWindowFlag::HAS_FOCUS == 0
            || !rect_contains(
                &self.item(item)?.window().rect().snapshot(),
                self.display_cursor_x,
                self.display_cursor_y,
            )
            || !is_activate_key(key)
        {
            return Ok(false);
        }
        let value = self.cvar_value(&cvar);
        self.options
            .cvars
            .set(&cvar, if value == 0.0 { "1" } else { "0" }, true);
        Ok(true)
    }

    /// Handle a multi key (`handleMultiKey`).
    fn handle_multi_key(&mut self, item: ItemState, key: i32) -> Result<bool, ClientError> {
        let cvar = self.item(item)?.cvar();
        let multi = match &self.item(item)?.behavior() {
            UiItemBehavior::Multi { multi } => multi.clone(),
            _ => None,
        };
        let (Some(cvar), Some(multi)) = (cvar, multi) else {
            return Ok(false);
        };
        if self.item(item)?.window().flags() & UiWindowFlag::HAS_FOCUS == 0
            || !rect_contains(
                &self.item(item)?.window().rect().snapshot(),
                self.display_cursor_x,
                self.display_cursor_y,
            )
            || !is_activate_key(key)
        {
            return Ok(false);
        }
        let count = multi.count();
        let mut current = 0i32;
        if multi.string_definition() {
            let value = self.cvar_buffer(&cvar)?;
            for index in 0..count.max(0) as usize {
                if equal_name(multi.string_value(index).as_deref(), &value) {
                    current = index as i32;
                    break;
                }
            }
        } else {
            let value = self.cvar_value(&cvar);
            for index in 0..count.max(0) as usize {
                if multi.number_value(index) == value {
                    current = index as i32;
                    break;
                }
            }
        }
        current += 1;
        if current < 0 || current >= count {
            current = 0;
        }
        if multi.string_definition() {
            let value = multi.string_value(current as usize);
            self.set_cvar(Some(&cvar), value.as_deref(), true);
        } else {
            let value = multi.number_value(current as usize);
            let integer = qvm_float_to_int(value);
            let text = if f(integer as f32) == value {
                game_format("%i", &[GameFormatArg::Int(integer)])?
            } else {
                game_format("%f", &[GameFormatArg::Float(value)])?
            };
            self.options.cvars.set(&cvar, &text, true);
        }
        Ok(true)
    }

    /// Publish an edit buffer to its cvar.
    fn publish_edit_buffer(&mut self, cvar: Option<&str>, buffer: &[u8; EDIT_BUFFER_LEN]) -> Result<(), ClientError> {
        let end = buffer.iter().position(|byte| *byte == 0);
        let Some(end) = end else {
            return Err(bad_ui("Item_TextField_HandleKey passes unterminated local char[1024]"));
        };
        let text: String = buffer[..end].iter().map(|byte| *byte as char).collect();
        self.set_cvar(cvar, Some(&text), true);
        Ok(())
    }

    /// Handle an edit-field key (`handleTextKey`).
    ///
    /// `Promise -> sync`.
    fn handle_text_key(&mut self, item: ItemState, key: i32) -> Result<bool, ClientError> {
        let cvar = self.item(item)?.cvar();
        let Some(cvar) = cvar else {
            return Ok(false);
        };
        let edit = self.item(item)?.edit_data();
        let Some(edit) = edit else {
            return Err(bad_ui(
                "Item_TextField_HandleKey dereferences NULL typeData at maxChars",
            ));
        };
        let value = self.cvar_buffer(&cvar)?;
        let mut buffer = [0u8; EDIT_BUFFER_LEN];
        let mut length = 0usize;
        for ch in value.chars() {
            if length >= buffer.len() {
                break;
            }
            buffer[length] = (ch as u32 & 255) as u8;
            length += 1;
        }
        if edit.max_chars() != 0 && length as i32 > edit.max_chars() {
            length = edit.max_chars() as usize;
        }
        let mut cursor = self.item(item)?.cursor_position();
        let kind = self.item(item)?.behavior().kind();
        if key & KEY_CHAR_FLAG != 0 {
            let key = key & !KEY_CHAR_FLAG;
            if key == 8 {
                if cursor > 0 {
                    edit_move(&mut buffer, cursor - 1, cursor, length as i32 + 1 - cursor)?;
                    cursor -= 1;
                    if cursor < edit.paint_offset() {
                        edit.set_paint_offset(edit.paint_offset() - (1));
                    }
                }
                self.publish_edit_buffer(Some(&cvar), &buffer)?;
                self.write_text_cursor(item, cursor, edit.paint_offset())?;
                return Ok(true);
            }
            if key < 32 {
                return Ok(true);
            }
            if kind == "numeric-field" && !(48..=57).contains(&key) {
                return Ok(false);
            }
            let overstrike = self.binding_host("edit overstrike")?.get_overstrike();
            if !overstrike {
                if length == 255 || (edit.max_chars() != 0 && length as i32 >= edit.max_chars()) {
                    return Ok(true);
                }
                edit_move(&mut buffer, cursor + 1, cursor, length as i32 + 1 - cursor)?;
            } else if edit.max_chars() != 0 && cursor >= edit.max_chars() {
                return Ok(true);
            }
            if cursor < 0 || cursor as usize >= buffer.len() {
                return Err(bad_ui("Item_TextField_HandleKey writes outside local char[1024]"));
            }
            buffer[cursor as usize] = (key & 255) as u8;
            self.publish_edit_buffer(Some(&cvar), &buffer)?;
            if cursor < length as i32 + 1 {
                cursor += 1;
                if edit.max_paint_chars() != 0 && cursor > edit.max_paint_chars() {
                    edit.set_paint_offset(edit.paint_offset() + (1));
                }
            }
            self.write_text_cursor(item, cursor, edit.paint_offset())?;
        } else if key == KeyCode::Delete as i32 || key == KeyCode::KeypadDelete as i32 {
            if cursor < length as i32 {
                edit_move(&mut buffer, cursor, cursor + 1, length as i32 - cursor)?;
                self.publish_edit_buffer(Some(&cvar), &buffer)?;
            }
            return Ok(true);
        } else if key == KeyCode::Right as i32 || key == KeyCode::KeypadRight as i32 {
            if edit.max_paint_chars() != 0 && cursor >= edit.max_paint_chars() && cursor < length as i32 {
                cursor += 1;
                edit.set_paint_offset(edit.paint_offset() + (1));
                self.write_text_cursor(item, cursor, edit.paint_offset())?;
                return Ok(true);
            }
            if cursor < length as i32 {
                cursor += 1;
            }
            self.write_text_cursor(item, cursor, edit.paint_offset())?;
            return Ok(true);
        } else if key == KeyCode::Left as i32 || key == KeyCode::KeypadLeft as i32 {
            if cursor > 0 {
                cursor -= 1;
            }
            if cursor < edit.paint_offset() {
                edit.set_paint_offset(edit.paint_offset() - (1));
            }
            self.write_text_cursor(item, cursor, edit.paint_offset())?;
            return Ok(true);
        } else if key == KeyCode::Home as i32 || key == KeyCode::KeypadHome as i32 {
            self.write_text_cursor(item, 0, 0)?;
            return Ok(true);
        } else if key == KeyCode::End as i32 || key == KeyCode::KeypadEnd as i32 {
            cursor = length as i32;
            if cursor > edit.max_paint_chars() {
                edit.set_paint_offset(length as i32 - edit.max_paint_chars());
            }
            self.write_text_cursor(item, cursor, edit.paint_offset())?;
            return Ok(true);
        } else if key == KeyCode::Insert as i32 || key == KeyCode::KeypadInsert as i32 {
            let overstrike = self.binding_host("edit overstrike")?.get_overstrike();
            self.binding_host("edit overstrike")?.set_overstrike(!overstrike);
            return Ok(true);
        }
        if key == KeyCode::Tab as i32 || key == KeyCode::Down as i32 || key == KeyCode::KeypadDown as i32 {
            let menu = self.menu_for(item)?;
            let next = self.focus_relative(menu, 1)?;
            if let Some(next) = next {
                let kind = self.item(next)?.behavior().kind();
                if kind == "edit-field" || kind == "numeric-field" {
                    self.editing_item = Some(next);
                }
            }
        }
        if key == KeyCode::Up as i32 || key == KeyCode::KeypadUp as i32 {
            let menu = self.menu_for(item)?;
            let previous = self.focus_relative(menu, -1)?;
            if let Some(previous) = previous {
                let kind = self.item(previous)?.behavior().kind();
                if kind == "edit-field" || kind == "numeric-field" {
                    self.editing_item = Some(previous);
                }
            }
        }
        Ok(key != KeyCode::Enter as i32 && key != KeyCode::KeypadEnter as i32 && key != KeyCode::Escape as i32)
    }

    /// Write back edit cursor state.
    fn write_text_cursor(&mut self, item: ItemState, cursor: i32, paint_offset: i32) -> Result<(), ClientError> {
        let definition = self.item(item)?;
        definition.set_cursor_position(cursor);
        if let Some(edit) = definition.edit_data() {
            edit.set_paint_offset(paint_offset);
        }
        Ok(())
    }

    /// Maximum list scroll (`listMaximum`).
    fn list_maximum(&mut self, item: ItemState) -> Result<i32, ClientError> {
        let (special, horizontal, rect) = {
            let definition = self.item(item)?;
            (
                definition.special(),
                definition.window().flags() & UiWindowFlag::HORIZONTAL != 0,
                definition.window().rect(),
            )
        };
        let list = self.item(item)?.list_data();
        let Some(list) = list else {
            return Err(bad_ui("Item_ListBox_MaxScroll dereferences NULL list data"));
        };
        let count = self.options.feeder.count(special);
        let element_size = if horizontal {
            list.element_width()
        } else {
            list.element_height()
        };
        let extent = if horizontal { rect.width() } else { rect.height() };
        Ok(0.max(qvm_float_to_int(f(f(f(count as f32) - f(extent / element_size)) + 1.0))))
    }

    /// Handle a list-box key (`handleListKey`).
    ///
    /// `Promise -> sync`.
    fn handle_list_key(&mut self, item: ItemState, key: i32, force: bool) -> Result<bool, ClientError> {
        let special = self.item(item)?.special();
        let count = self.options.feeder.count(special);
        if !force
            && (!rect_contains(
                &self.item(item)?.window().rect().snapshot(),
                self.display_cursor_x,
                self.display_cursor_y,
            ) || self.item(item)?.window().flags() & UiWindowFlag::HAS_FOCUS == 0)
        {
            return Ok(false);
        }
        let maximum = self.list_maximum(item)?;
        let horizontal = self.item(item)?.window().flags() & UiWindowFlag::HORIZONTAL != 0;
        let list = self.item(item)?.list_data();
        let Some(list) = list else {
            return Err(bad_ui("Item_ListBox_HandleKey dereferences NULL list data"));
        };
        let rect = self.item(item)?.window().rect();
        let view = qvm_float_to_int(f(if horizontal {
            rect.width() / list.element_width()
        } else {
            rect.height() / list.element_height()
        }));
        let backward = if horizontal {
            key == KeyCode::Left as i32 || key == KeyCode::KeypadLeft as i32
        } else {
            key == KeyCode::Up as i32 || key == KeyCode::KeypadUp as i32
        };
        let forward = if horizontal {
            key == KeyCode::Right as i32 || key == KeyCode::KeypadRight as i32
        } else {
            key == KeyCode::Down as i32 || key == KeyCode::KeypadDown as i32
        };
        if backward {
            if !list.not_selectable() {
                list.set_cursor_position(list.cursor_position() - (1));
                if list.cursor_position() < 0 {
                    list.set_cursor_position(0);
                }
                if list.cursor_position() < list.start_position() {
                    list.set_start_position(list.cursor_position());
                }
                if list.cursor_position() >= list.start_position() + view {
                    list.set_start_position(list.cursor_position() - view + 1);
                }
                self.write_list(item, &list);
                self.select_list(item)?;
            } else {
                list.set_start_position(list.start_position() - (1));
                if list.start_position() < 0 {
                    list.set_start_position(0);
                }
                self.write_list(item, &list);
            }
            return Ok(true);
        }
        if forward {
            if !list.not_selectable() {
                list.set_cursor_position(list.cursor_position() + (1));
                if list.cursor_position() < list.start_position() {
                    list.set_start_position(list.cursor_position());
                }
                if list.cursor_position() >= count {
                    list.set_cursor_position(count - 1);
                }
                if list.cursor_position() >= list.start_position() + view {
                    list.set_start_position(list.cursor_position() - view + 1);
                }
                self.write_list(item, &list);
                self.select_list(item)?;
            } else {
                list.set_start_position(list.start_position() + (1));
                let limit = if horizontal { count - 1 } else { maximum };
                if list.start_position() > limit {
                    list.set_start_position(limit);
                }
                self.write_list(item, &list);
            }
            return Ok(true);
        }
        if key == KeyCode::Mouse1 as i32 || key == KeyCode::Mouse2 as i32 {
            let flags = self.item(item)?.window().flags();
            if flags & UiWindowFlag::LIST_LEFT_ARROW != 0 {
                list.set_start_position(list.start_position() - (1));
                if list.start_position() < 0 {
                    list.set_start_position(0);
                }
                self.write_list(item, &list);
            } else if flags & UiWindowFlag::LIST_RIGHT_ARROW != 0 {
                list.set_start_position(list.start_position() + (1));
                if list.start_position() > maximum {
                    list.set_start_position(maximum);
                }
                self.write_list(item, &list);
            } else if flags & UiWindowFlag::LIST_PAGE_UP != 0 {
                list.set_start_position(list.start_position() - (view));
                if list.start_position() < 0 {
                    list.set_start_position(0);
                }
                self.write_list(item, &list);
            } else if flags & UiWindowFlag::LIST_PAGE_DOWN != 0 {
                list.set_start_position(list.start_position() + (view));
                if list.start_position() > maximum {
                    list.set_start_position(maximum);
                }
                self.write_list(item, &list);
            } else if flags & UiWindowFlag::LIST_THUMB == 0 {
                if self.real_time < self.last_list_box_click_time {
                    if let Some(script) = list.double_click() {
                        let menu = self.menu_for(item)?;
                        self.run_script(ScriptOwner { menu, item: Some(item) }, &script)?;
                    }
                }
                self.last_list_box_click_time = self.real_time.wrapping_add(DOUBLE_CLICK_DELAY);
                if self.item(item)?.cursor_position() != list.cursor_position() {
                    self.select_list(item)?;
                }
            }
            return Ok(true);
        }
        if key == KeyCode::Home as i32 || key == KeyCode::KeypadHome as i32 {
            list.set_start_position(0);
            self.write_list(item, &list);
            return Ok(true);
        }
        if key == KeyCode::End as i32 || key == KeyCode::KeypadEnd as i32 {
            list.set_start_position(maximum);
            self.write_list(item, &list);
            return Ok(true);
        }
        let page_up = key == KeyCode::PageUp as i32 || key == KeyCode::KeypadPageUp as i32;
        let page_down = key == KeyCode::PageDown as i32 || key == KeyCode::KeypadPageDown as i32;
        if page_up || page_down {
            let amount = if page_up { -view } else { view };
            if !list.not_selectable() {
                list.set_cursor_position(list.cursor_position() + (amount));
                if page_up && list.cursor_position() < 0 {
                    list.set_cursor_position(0);
                }
                if list.cursor_position() < list.start_position() {
                    list.set_start_position(list.cursor_position());
                }
                if page_down && list.cursor_position() >= count {
                    list.set_cursor_position(count - 1);
                }
                if list.cursor_position() >= list.start_position() + view {
                    list.set_start_position(list.cursor_position() - view + 1);
                }
                self.write_list(item, &list);
                self.select_list(item)?;
            } else {
                list.set_start_position(list.start_position() + (amount));
                if page_up && list.start_position() < 0 {
                    list.set_start_position(0);
                }
                if page_down && list.start_position() > maximum {
                    list.set_start_position(maximum);
                }
                self.write_list(item, &list);
            }
            return Ok(true);
        }
        Ok(false)
    }

    /// Write back list-box positions.
    fn write_list(&mut self, item: ItemState, list: &UiListBoxDefinition) {
        let Ok(definition) = self.item(item) else {
            return;
        };
        if let Some(owned) = definition.list_data() {
            owned.set_start_position(list.start_position());
            owned.set_end_position(list.end_position());
            owned.set_draw_padding(list.draw_padding());
            owned.set_cursor_position(list.cursor_position());
        }
    }

    /// Record a list selection (`selectList`).
    ///
    /// `Promise -> sync`.
    fn select_list(&mut self, item: ItemState) -> Result<(), ClientError> {
        let (special, cursor) = {
            let definition = self.item(item)?;
            let list = definition.list_data();
            let Some(list) = list else {
                return Err(bad_ui("Item_ListBox select dereferences NULL list data"));
            };
            (definition.special(), list.cursor_position())
        };
        self.menu_item(item.menu_index, item.item_index)?
            .set_cursor_position(cursor);
        self.options.feeder.select(special, cursor);
        Ok(())
    }

    /// List thumb position (`listThumbPosition`).
    fn list_thumb_position(&mut self, item: ItemState) -> Result<i32, ClientError> {
        let (special, horizontal, rect) = {
            let definition = self.item(item)?;
            (
                definition.special(),
                definition.window().flags() & UiWindowFlag::HORIZONTAL != 0,
                definition.window().rect(),
            )
        };
        let _ = special;
        let maximum = self.list_maximum(item)?;
        let list = self.item(item)?.list_data();
        let Some(list) = list else {
            return Err(bad_ui("Item_ListBox_ThumbPosition dereferences NULL list data"));
        };
        let extent = if horizontal { rect.width() } else { rect.height() };
        let size = f(f(extent - SCROLLBAR_SIZE * 2.0) - 2.0);
        let step = if maximum > 0 {
            f(f(size - SCROLLBAR_SIZE) / f(maximum as f32))
        } else {
            0.0
        };
        let base = if horizontal { rect.x() } else { rect.y() };
        Ok(qvm_float_to_int(f(
            f(f(base + 1.0) + SCROLLBAR_SIZE) + f(step * f(list.start_position() as f32))
        )))
    }

    /// List thumb draw position (`listThumbDrawPosition`).
    fn list_thumb_draw_position(&mut self, item: ItemState) -> Result<i32, ClientError> {
        if let Some(captured) = self.capture.item() {
            if captured == item {
                let (horizontal, rect) = {
                    let definition = self.item(item)?;
                    (
                        definition.window().flags() & UiWindowFlag::HORIZONTAL != 0,
                        definition.window().rect(),
                    )
                };
                let start = if horizontal { rect.x() } else { rect.y() };
                let extent = if horizontal { rect.width() } else { rect.height() };
                let minimum = qvm_float_to_int(f(f(start + SCROLLBAR_SIZE) + 1.0));
                let maximum = qvm_float_to_int(f(f(f(start + extent) - 2.0 * SCROLLBAR_SIZE) - 1.0));
                let cursor = if horizontal {
                    self.display_cursor_x
                } else {
                    self.display_cursor_y
                };
                if cursor >= f(f(minimum as f32) + SCROLLBAR_SIZE / 2.0)
                    && cursor <= f(f(maximum as f32) + SCROLLBAR_SIZE / 2.0)
                {
                    return Ok(qvm_float_to_int(f(cursor - SCROLLBAR_SIZE / 2.0)));
                }
            }
        }
        self.list_thumb_position(item)
    }

    /// List scrollbar hit test (`listHit`).
    fn list_hit(&mut self, item: ItemState, x: f32, y: f32) -> Result<i32, ClientError> {
        let special = self.item(item)?.special();
        self.options.feeder.count(special);
        let (horizontal, rect) = {
            let definition = self.item(item)?;
            (
                definition.window().flags() & UiWindowFlag::HORIZONTAL != 0,
                definition.window().rect(),
            )
        };
        if horizontal {
            let mut part = UiRect {
                x: rect.x(),
                y: f(f(rect.y() + rect.height()) - SCROLLBAR_SIZE),
                width: SCROLLBAR_SIZE,
                height: SCROLLBAR_SIZE,
            };
            if rect_contains(&part, x, y) {
                return Ok(UiWindowFlag::LIST_LEFT_ARROW);
            }
            part.x = f(f(rect.x() + rect.width()) - SCROLLBAR_SIZE);
            if rect_contains(&part, x, y) {
                return Ok(UiWindowFlag::LIST_RIGHT_ARROW);
            }
            let thumb = self.list_thumb_position(item)?;
            part.x = thumb as f32;
            if rect_contains(&part, x, y) {
                return Ok(UiWindowFlag::LIST_THUMB);
            }
            part.x = f(rect.x() + SCROLLBAR_SIZE);
            part.width = f(f(thumb as f32) - f(rect.x() + SCROLLBAR_SIZE));
            if rect_contains(&part, x, y) {
                return Ok(UiWindowFlag::LIST_PAGE_UP);
            }
            part.x = f(f(thumb as f32) + SCROLLBAR_SIZE);
            part.width = f(f(rect.x() + rect.width()) - SCROLLBAR_SIZE);
            if rect_contains(&part, x, y) {
                return Ok(UiWindowFlag::LIST_PAGE_DOWN);
            }
        } else {
            let mut part = UiRect {
                x: f(f(rect.x() + rect.width()) - SCROLLBAR_SIZE),
                y: rect.y(),
                width: SCROLLBAR_SIZE,
                height: SCROLLBAR_SIZE,
            };
            if rect_contains(&part, x, y) {
                return Ok(UiWindowFlag::LIST_LEFT_ARROW);
            }
            part.y = f(f(rect.y() + rect.height()) - SCROLLBAR_SIZE);
            if rect_contains(&part, x, y) {
                return Ok(UiWindowFlag::LIST_RIGHT_ARROW);
            }
            let thumb = self.list_thumb_position(item)?;
            part.y = thumb as f32;
            if rect_contains(&part, x, y) {
                return Ok(UiWindowFlag::LIST_THUMB);
            }
            part.y = f(rect.y() + SCROLLBAR_SIZE);
            part.height = f(f(thumb as f32) - f(rect.y() + SCROLLBAR_SIZE));
            if rect_contains(&part, x, y) {
                return Ok(UiWindowFlag::LIST_PAGE_UP);
            }
            part.y = f(f(thumb as f32) + SCROLLBAR_SIZE);
            part.height = f(f(rect.y() + rect.height()) - SCROLLBAR_SIZE);
            if rect_contains(&part, x, y) {
                return Ok(UiWindowFlag::LIST_PAGE_DOWN);
            }
        }
        Ok(0)
    }

    /// List mouse-enter (`listMouseEnter`).
    fn list_mouse_enter(&mut self, item: ItemState, x: f32, y: f32) -> Result<(), ClientError> {
        let hit = self.list_hit(item, x, y)?;
        {
            let window = self.item(item)?.window();
            let mut flags = window.flags();
            flags &= !(UiWindowFlag::LIST_LEFT_ARROW
                | UiWindowFlag::LIST_RIGHT_ARROW
                | UiWindowFlag::LIST_THUMB
                | UiWindowFlag::LIST_PAGE_UP
                | UiWindowFlag::LIST_PAGE_DOWN);
            flags |= hit;
            window.set_flags(flags);
            let controls = UiWindowFlag::LIST_LEFT_ARROW
                | UiWindowFlag::LIST_RIGHT_ARROW
                | UiWindowFlag::LIST_THUMB
                | UiWindowFlag::LIST_PAGE_UP
                | UiWindowFlag::LIST_PAGE_DOWN;
            if flags & controls != 0 {
                return Ok(());
            }
        }
        let list = self.item(item)?.list_data();
        let Some(list) = list else {
            return Err(bad_ui("Item_ListBox_MouseEnter dereferences NULL list data"));
        };
        let (horizontal, rect) = {
            let definition = self.item(item)?;
            (
                definition.window().flags() & UiWindowFlag::HORIZONTAL != 0,
                definition.window().rect(),
            )
        };
        if horizontal {
            if list.element_style() != 1 {
                return Ok(());
            }
            let part = UiRect {
                x: rect.x(),
                y: rect.y(),
                width: f(rect.width() - list.draw_padding() as f32),
                height: f(rect.height() - SCROLLBAR_SIZE),
            };
            if rect_contains(&part, x, y) {
                list.set_cursor_position(
                    list.end_position()
                        .min(qvm_float_to_int(f(f(x - part.x) / list.element_width())) + list.start_position()),
                );
                self.write_list(item, &list);
            }
        } else {
            let part = UiRect {
                x: rect.x(),
                y: rect.y(),
                width: f(rect.width() - SCROLLBAR_SIZE),
                height: f(rect.height() - list.draw_padding() as f32),
            };
            if rect_contains(&part, x, y) {
                list.set_cursor_position(
                    list.end_position().min(
                        qvm_float_to_int(f(f(f(y - 2.0) - part.y) / list.element_height())) + list.start_position(),
                    ),
                );
                self.write_list(item, &list);
            }
        }
        Ok(())
    }

    /// Slider bar x (`sliderX`).
    fn slider_x(&self, item: ItemState) -> Result<f32, ClientError> {
        let definition = self.item(item)?;
        Ok(match definition.text().as_ref() {
            None => definition.window().rect().x(),
            Some(_) => f(f(definition.text_rect().x() + definition.text_rect().width()) + 8.0),
        })
    }

    /// Slider thumb position (`sliderThumbPosition`).
    fn slider_thumb_position(&mut self, item: ItemState) -> Result<f32, ClientError> {
        let edit = self.item(item)?.edit_data();
        let x = self.slider_x(item)?;
        let cvar = self.item(item)?.cvar();
        if edit.is_none() && cvar.is_some() {
            return Ok(x);
        }
        let Some(cvar) = cvar else {
            return Err(bad_ui("Item_Slider_ThumbPosition hashes NULL cvar in Cvar_FindVar"));
        };
        let value = self.cvar_value(&cvar);
        let Some(edit) = edit else {
            return Err(bad_ui("Item_Slider_ThumbPosition dereferences NULL edit data"));
        };
        let mut value = value.clamp(edit.minimum(), edit.maximum());
        if edit.minimum() > edit.maximum() {
            value = if value < edit.minimum() {
                edit.minimum()
            } else {
                edit.maximum()
            };
        }
        let range = f(edit.maximum() - edit.minimum());
        value = f(value - edit.minimum());
        value = f(value / range);
        value = f(value * SLIDER_WIDTH);
        Ok(f(x + value))
    }

    /// Handle a slider key (`handleSliderKey`).
    fn handle_slider_key(&mut self, item: ItemState, key: i32) -> Result<bool, ClientError> {
        let cvar = self.item(item)?.cvar();
        if let Some(cvar) = cvar {
            if self.item(item)?.window().flags() & UiWindowFlag::HAS_FOCUS != 0
                && rect_contains(
                    &self.item(item)?.window().rect().snapshot(),
                    self.display_cursor_x,
                    self.display_cursor_y,
                )
                && is_activate_key(key)
            {
                let edit = self.item(item)?.edit_data();
                if let Some(edit) = edit {
                    let x = self.slider_x(item)?;
                    let rect = self.item(item)?.window().rect();
                    let test = UiRect {
                        x: f(x - SLIDER_THUMB_WIDTH / 2.0),
                        width: f(SLIDER_WIDTH + SLIDER_THUMB_WIDTH / 2.0),
                        ..rect.snapshot()
                    };
                    if rect_contains(&test, self.display_cursor_x, self.display_cursor_y) {
                        let work = f(self.display_cursor_x - x);
                        let value = f(f(f(work / SLIDER_WIDTH) * f(edit.maximum() - edit.minimum())) + edit.minimum());
                        let text = game_format("%f", &[GameFormatArg::Float(value)])?;
                        self.options.cvars.set(&cvar, &text, true);
                        return Ok(true);
                    }
                }
            }
        }
        self.print("slider handle key exit\n");
        Ok(false)
    }

    /// Start a pointer capture (`startCapture`).
    fn start_capture(&mut self, item: ItemState, key: i32) -> Result<(), ClientError> {
        let kind = self.item(item)?.behavior().kind();
        if kind == "list-box" || kind == "edit-field" || kind == "numeric-field" {
            let flags = self.list_hit(item, self.display_cursor_x, self.display_cursor_y)?;
            if flags & (UiWindowFlag::LIST_LEFT_ARROW | UiWindowFlag::LIST_RIGHT_ARROW) != 0 {
                self.next_scroll_time = self.real_time.wrapping_add(500);
                self.next_scroll_adjust_time = self.real_time.wrapping_add(150);
                self.scroll_adjust_value = 500;
                self.capture = CaptureState::ListAuto {
                    item,
                    key,
                    x_start: self.display_cursor_x,
                    y_start: self.display_cursor_y,
                };
            } else if flags & UiWindowFlag::LIST_THUMB != 0 {
                self.capture = CaptureState::ListThumb {
                    item,
                    key,
                    x_start: self.display_cursor_x,
                    y_start: self.display_cursor_y,
                };
            }
        } else if kind == "slider" {
            let thumb = self.slider_thumb_position(item)?;
            let rect = self.item(item)?.window().rect();
            let part = UiRect {
                x: f(thumb - SLIDER_THUMB_WIDTH / 2.0),
                y: f(rect.y() - 2.0),
                width: SLIDER_THUMB_WIDTH,
                height: 20.0,
            };
            if rect_contains(&part, self.display_cursor_x, self.display_cursor_y) {
                self.capture = CaptureState::SliderThumb {
                    item,
                    key,
                    x_start: self.display_cursor_x,
                    y_start: self.display_cursor_y,
                };
            }
        }
        Ok(())
    }

    /// Handle a binding key (`handleBindKey`).
    fn handle_bind_key(&mut self, item: ItemState, key: i32, down: bool) -> Result<bool, ClientError> {
        if rect_contains(
            &self.item(item)?.window().rect().snapshot(),
            self.display_cursor_x,
            self.display_cursor_y,
        ) && !self.waiting_for_key
        {
            if down && (key == KeyCode::Mouse1 as i32 || key == KeyCode::Enter as i32) {
                self.waiting_for_key = true;
                self.binding_item = Some(item);
            }
            return Ok(true);
        }
        if !self.waiting_for_key || self.binding_item.is_none() {
            return Ok(true);
        }
        if key & KEY_CHAR_FLAG != 0 {
            return Ok(true);
        }
        if key == KeyCode::Escape as i32 {
            self.waiting_for_key = false;
            return Ok(true);
        }
        let target = self.binding_by_name(self.item(item)?.cvar().as_deref());
        if key == KeyCode::Backspace as i32 {
            if let Some(target) = target {
                self.bindings[target].first = NO_BINDING;
                self.bindings[target].second = NO_BINDING;
            }
            self.write_bindings()?;
            self.waiting_for_key = false;
            self.binding_item = None;
            return Ok(true);
        }
        if key == 96 {
            return Ok(true);
        }
        if key != -1 {
            for binding in &mut self.bindings {
                if binding.second == key {
                    binding.second = NO_BINDING;
                }
                if binding.first == key {
                    binding.first = binding.second;
                    binding.second = NO_BINDING;
                }
            }
        }
        if let Some(target) = target {
            if key == -1 {
                let (first, second) = (self.bindings[target].first, self.bindings[target].second);
                if first != NO_BINDING {
                    self.binding_host("clear binding")?.set_binding(first, "");
                    self.bindings[target].first = NO_BINDING;
                }
                if second != NO_BINDING {
                    self.binding_host("clear binding")?.set_binding(second, "");
                    self.bindings[target].second = NO_BINDING;
                }
            } else if self.bindings[target].first == NO_BINDING {
                self.bindings[target].first = key;
            } else if self.bindings[target].first != key && self.bindings[target].second == NO_BINDING {
                self.bindings[target].second = key;
            } else {
                let (first, second) = (self.bindings[target].first, self.bindings[target].second);
                self.binding_host("replace binding")?.set_binding(first, "");
                self.binding_host("replace binding")?.set_binding(second, "");
                self.bindings[target].first = key;
                self.bindings[target].second = NO_BINDING;
            }
        }
        self.write_bindings()?;
        self.waiting_for_key = false;
        Ok(true)
    }

    /// Run capture timers (`runCapture`).
    ///
    /// `Promise -> sync`.
    fn run_capture(&mut self) -> Result<(), ClientError> {
        let capture = self.capture;
        match capture {
            CaptureState::Idle => return Ok(()),
            CaptureState::SliderThumb { item, .. } => {
                let edit = self.item(item)?.edit_data();
                let x = self.slider_x(item)?;
                let cursor = self.display_cursor_x.clamp(x, f(x + SLIDER_WIDTH));
                let Some(edit) = edit else {
                    return Err(bad_ui("Scroll_Slider_ThumbFunc dereferences NULL edit data"));
                };
                let value = f(f(f(f(cursor - x) / SLIDER_WIDTH) * f(edit.maximum() - edit.minimum())) + edit.minimum());
                let text = game_format("%f", &[GameFormatArg::Float(value)])?;
                let cvar = self.item(item)?.cvar();
                self.set_cvar(cvar.as_deref(), Some(&text), true);
                return Ok(());
            }
            CaptureState::ListThumb { item, .. } => {
                let horizontal = self.item(item)?.window().flags() & UiWindowFlag::HORIZONTAL != 0;
                if horizontal {
                    if self.display_cursor_x
                        == match capture {
                            CaptureState::ListThumb { x_start, .. } => x_start,
                            _ => 0.0,
                        }
                    {
                        return Ok(());
                    }
                    let rect = self.item(item)?.window().rect();
                    let width = f(f(rect.width() - SCROLLBAR_SIZE * 2.0) - 2.0);
                    let start = f(f(rect.x() + SCROLLBAR_SIZE) + 1.0);
                    let maximum = self.list_maximum(item)?;
                    let denominator = f(width - SCROLLBAR_SIZE);
                    let list = self.item(item)?.list_data();
                    let Some(list) = list else {
                        return Err(bad_ui("Scroll_ListBox_ThumbFunc dereferences NULL list data"));
                    };
                    list.set_start_position(0.max(maximum.min(qvm_float_to_int(f(f(f(f(
                        self.display_cursor_x - start
                    ) - SCROLLBAR_SIZE / 2.0)
                        * f(maximum as f32))
                        / denominator)))));
                    self.write_list(item, &list);
                    if let CaptureState::ListThumb { x_start, .. } = &mut self.capture {
                        *x_start = self.display_cursor_x;
                    }
                } else {
                    let y_start = match capture {
                        CaptureState::ListThumb { y_start, .. } => y_start,
                        _ => 0.0,
                    };
                    if self.display_cursor_y != y_start {
                        let rect = self.item(item)?.window().rect();
                        let height = f(f(rect.height() - SCROLLBAR_SIZE * 2.0) - 2.0);
                        let start = f(f(rect.y() + SCROLLBAR_SIZE) + 1.0);
                        let maximum = self.list_maximum(item)?;
                        let denominator = f(height - SCROLLBAR_SIZE);
                        let list = self.item(item)?.list_data();
                        let Some(list) = list else {
                            return Err(bad_ui("Scroll_ListBox_ThumbFunc dereferences NULL list data"));
                        };
                        list.set_start_position(0.max(maximum.min(qvm_float_to_int(f(f(f(f(
                            self.display_cursor_y - start,
                        ) - SCROLLBAR_SIZE
                            / 2.0)
                            * f(maximum as f32))
                            / denominator)))));
                        self.write_list(item, &list);
                        if let CaptureState::ListThumb { y_start, .. } = &mut self.capture {
                            *y_start = self.display_cursor_y;
                        }
                    }
                }
            }
            CaptureState::ListAuto { .. } => {}
        }
        let (item, key) = match capture {
            CaptureState::ListAuto { item, key, .. } | CaptureState::ListThumb { item, key, .. } => (item, key),
            _ => return Ok(()),
        };
        if self.real_time > self.next_scroll_time {
            self.handle_list_key(item, key, false)?;
            self.next_scroll_time = self.real_time.wrapping_add(self.scroll_adjust_value);
        }
        if self.real_time > self.next_scroll_adjust_time {
            self.next_scroll_adjust_time = self.real_time.wrapping_add(150);
            if self.scroll_adjust_value > 20 {
                self.scroll_adjust_value -= 40;
            }
        }
        Ok(())
    }

    /// Paint a menu (`paintMenu`).
    ///
    /// `Promise -> sync`.
    fn paint_menu(&mut self, menu: usize, draw: &mut Draw2D, force: bool) -> Result<(), ClientError> {
        if self.menu(menu).window().flags() & UiWindowFlag::VISIBLE == 0 && !force {
            return Ok(());
        }
        if self.menu(menu).window().owner_draw_flags() != 0
            && !self
                .options
                .owner_draw
                .visible(self.menu(menu).window().owner_draw_flags())
        {
            return Ok(());
        }
        if force {
            self.definitions.menus[menu]
                .window()
                .set_flags(self.definitions.menus[menu].window().flags() | (UiWindowFlag::FORCED));
        }
        if self.menu(menu).full_screen() != 0 {
            let background = self.background_or_zero(&self.menu(menu).window())?;
            draw.draw_handle_pic(
                Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 640.0,
                    height: 480.0,
                },
                background,
            );
        }
        let (fade_amount, fade_clamp, fade_cycle) = {
            let menu_def = self.menu(menu);
            (menu_def.fade_amount(), menu_def.fade_clamp(), menu_def.fade_cycle())
        };
        self.paint_window(menu, None, fade_amount, fade_clamp, fade_cycle, draw)?;
        for item in 0..self.menu(menu).item_count().max(0) as usize {
            self.paint_item(menu, item, draw)?;
        }
        if self.debug {
            let rect = self.menu(menu).window().rect().snapshot();
            self.draw_rect(
                draw,
                &rect,
                1.0,
                Vec4 {
                    x: 1.0,
                    y: 0.0,
                    z: 1.0,
                    w: 1.0,
                },
            );
        }
        Ok(())
    }

    /// Paint a window (`paintWindow`).
    ///
    /// `Promise -> sync`.
    #[allow(clippy::too_many_arguments)]
    fn paint_window(
        &mut self,
        menu: usize,
        item: Option<usize>,
        fade_amount: f32,
        fade_clamp: f32,
        fade_cycle: i32,
        draw: &mut Draw2D,
    ) -> Result<(), ClientError> {
        let window = match item {
            Some(item) => self.menu_item(menu, item)?.window(),
            None => self.definitions.menus[menu].window(),
        };
        if self.debug {
            self.draw_rect(
                draw,
                &window.rect().snapshot(),
                1.0,
                Vec4 {
                    x: 1.0,
                    y: 1.0,
                    z: 1.0,
                    w: 1.0,
                },
            );
        }
        if window.style() == 0 && window.border() == 0 {
            return Ok(());
        }
        let mut fill = window.rect().snapshot();
        if window.border() != 0 {
            fill = UiRect {
                x: f(fill.x + window.border_size()),
                y: f(fill.y + window.border_size()),
                width: f(fill.width - f(window.border_size() + 1.0)),
                height: f(fill.height - f(window.border_size() + 1.0)),
            };
        }
        let mut team_color = None;
        if window.style() == 1 {
            if self.has_window_background(&window)? {
                let real_time = self.real_time;
                let mut target = match item {
                    Some(item) => self.menu_item(menu, item)?.window(),
                    None => self.definitions.menus[menu].window(),
                };
                Self::fade_window(real_time, &mut target, false, fade_clamp, fade_cycle, true, fade_amount);
                let window = match item {
                    Some(item) => self.menu_item(menu, item)?.window(),
                    None => self.definitions.menus[menu].window(),
                };
                let background = self.background_or_zero(&window)?;
                draw.set_color(Some(window.back_color().snapshot()));
                draw.draw_handle_pic(Rect::from(&fill), background);
                draw.set_color(None);
            } else {
                draw.fill_rect(
                    Rect::from(&fill),
                    window.back_color().snapshot(),
                    self.options.widget_assets.white_shader,
                );
            }
        } else if window.style() == 2 {
            self.gradient(draw, &fill, window.back_color().snapshot());
        } else if window.style() == 3 {
            if window.flags() & UiWindowFlag::FORE_COLOR_SET != 0 {
                draw.set_color(Some(window.fore_color().snapshot()));
            }
            let background = self.background_or_zero(&window)?;
            draw.draw_handle_pic(Rect::from(&fill), background);
            draw.set_color(None);
        } else if window.style() == 4 {
            let color = (self.options.get_team_color)();
            team_color = Some(color);
            draw.fill_rect(Rect::from(&fill), color, self.options.widget_assets.white_shader);
        } else if window.style() == 5 {
            self.paint_cinematic(menu, item, fill, draw)?;
        }
        if window.border() == 1 {
            if window.style() == 4 {
                if let Some(team) = team_color {
                    let color = if team.x > 0.0 {
                        Vec4 {
                            x: 1.0,
                            y: 0.5,
                            z: 0.5,
                            w: 1.0,
                        }
                    } else {
                        Vec4 {
                            x: 0.5,
                            y: 0.5,
                            z: 1.0,
                            w: 1.0,
                        }
                    };
                    self.draw_rect(draw, &window.rect().snapshot(), window.border_size(), color);
                }
            } else {
                self.draw_rect(
                    draw,
                    &window.rect().snapshot(),
                    window.border_size(),
                    window.border_color().snapshot(),
                );
            }
        } else if window.border() == 2 {
            draw.set_color(Some(window.border_color().snapshot()));
            draw_cg_top_bottom(
                draw,
                &Rect::from(&window.rect().snapshot()),
                window.border_size(),
                self.options.widget_assets.white_shader,
            );
            draw.set_color(None);
        } else if window.border() == 3 {
            draw.set_color(Some(window.border_color().snapshot()));
            draw_cg_sides(
                draw,
                &Rect::from(&window.rect().snapshot()),
                window.border_size(),
                self.options.widget_assets.white_shader,
            );
            draw.set_color(None);
        } else if window.border() == 4 {
            let top = UiRect {
                height: window.border_size(),
                ..window.rect().snapshot()
            };
            self.gradient(draw, &top, window.border_color().snapshot());
            let bottom = UiRect {
                y: f(f(window.rect().y() + window.rect().height()) - 1.0),
                height: window.border_size(),
                ..window.rect().snapshot()
            };
            self.gradient(draw, &bottom, window.border_color().snapshot());
        }
        Ok(())
    }

    /// Paint a CG rectangle outline (`drawRect`).
    fn draw_rect(&self, draw: &mut Draw2D, rect: &UiRect, size: f32, color: Vec4) {
        draw_cg_rect(
            draw,
            &Rect::from(rect),
            size,
            color,
            self.options.widget_assets.white_shader,
        );
    }

    /// Paint a gradient bar (`gradient`).
    fn gradient(&self, draw: &mut Draw2D, rect: &UiRect, color: Vec4) {
        draw.set_color(Some(color));
        draw.draw_handle_pic(Rect::from(rect), self.options.widget_assets.gradient_bar);
        draw.set_color(None);
    }

    /// Paint a cinematic window (`paintCinematic`).
    ///
    /// `Promise -> sync`.
    fn paint_cinematic(
        &mut self,
        menu: usize,
        item: Option<usize>,
        rect: UiRect,
        draw: &mut Draw2D,
    ) -> Result<(), ClientError> {
        let handle = match item {
            Some(item) => self.menu_item(menu, item)?.window().cinematic_handle(),
            None => self.definitions.menus[menu].window().cinematic_handle(),
        };
        if handle == -1 {
            let path = match item {
                Some(item) => self.menu_item(menu, item)?.window().cinematic(),
                None => self.definitions.menus[menu].window().cinematic(),
            };
            let Some(path) = path else {
                return Err(bad_ui("cinematic window has no cinematic path"));
            };
            let asset = self.cinematic_asset(&path);
            let instance = self.options.cinematics.play(&asset, &rect);
            let target = match item {
                Some(item) => self.menu_item(menu, item)?.window(),
                None => self.definitions.menus[menu].window(),
            };
            target.set_cinematic_handle(instance.map(|found| found.handle).unwrap_or(-2));
        }
        let handle = match item {
            Some(item) => self.menu_item(menu, item)?.window().cinematic_handle(),
            None => self.definitions.menus[menu].window().cinematic_handle(),
        };
        if handle >= 0 {
            let time = self.real_time;
            self.options.cinematics.run(handle, time);
            self.options.cinematics.draw(handle, &rect, draw);
        }
        Ok(())
    }

    /// Advance a fade (`fade`).
    fn fade_window(
        real_time: i32,
        window: &mut UiWindowDefinition,
        fore: bool,
        clamp: f32,
        cycle: i32,
        clear_flags: bool,
        amount: f32,
    ) {
        if window.flags() & (UiWindowFlag::FADING_OUT | UiWindowFlag::FADING_IN) == 0 || real_time <= window.next_time()
        {
            return;
        }
        window.set_next_time(real_time.wrapping_add(cycle));
        let color = if fore {
            &mut window.fore_color()
        } else {
            &mut window.back_color()
        };
        if window.flags() & UiWindowFlag::FADING_OUT != 0 {
            color.set_w(f(color.w() - amount));
            if clear_flags && color.w() <= 0.0 {
                window.set_flags(window.flags() & (!(UiWindowFlag::FADING_OUT | UiWindowFlag::VISIBLE)));
            }
        } else {
            color.set_w(f(color.w() + amount));
            if color.w() >= clamp {
                color.set_w(f(clamp));
                if clear_flags {
                    window.set_flags(window.flags() & (!UiWindowFlag::FADING_IN));
                }
            }
        }
    }

    /// Paint an item (`paintItem`).
    ///
    /// `Promise -> sync`.
    fn paint_item(&mut self, menu: usize, item: usize, draw: &mut Draw2D) -> Result<(), ClientError> {
        let target = ItemState {
            menu_index: menu,
            item_index: item,
        };
        self.advance_item_animation(target)?;
        if self.item(target)?.window().owner_draw_flags() != 0 {
            let flags = self.item(target)?.window().owner_draw_flags();
            let visible = self.options.owner_draw.visible(flags);
            if visible {
                self.menu_item(menu, item)?
                    .window()
                    .set_flags(self.menu_item(menu, item)?.window().flags() | UiWindowFlag::VISIBLE);
            } else {
                self.menu_item(menu, item)?
                    .window()
                    .set_flags(self.menu_item(menu, item)?.window().flags() & !UiWindowFlag::VISIBLE);
            }
        }
        if !self.item_passes_cvar(target, "show")? || self.item(target)?.window().flags() & UiWindowFlag::VISIBLE == 0 {
            return Ok(());
        }
        let parent = self.parent_menu(target)?;
        let Some(parent) = parent else {
            return Err(bad_ui("Item_Paint dereferences a NULL parent menu at fadeAmount"));
        };
        let (fade_amount, fade_clamp, fade_cycle) = {
            let menu_def = self.menu(parent);
            (menu_def.fade_amount(), menu_def.fade_clamp(), menu_def.fade_cycle())
        };
        self.paint_window(menu, Some(item), fade_amount, fade_clamp, fade_cycle, draw)?;
        if self.debug {
            let rect = self.corrected_text_rect(target)?;
            self.draw_rect(
                draw,
                &rect,
                1.0,
                Vec4 {
                    x: 0.0,
                    y: 1.0,
                    z: 0.0,
                    w: 1.0,
                },
            );
        }
        match self.item(target)?.behavior().kind() {
            "owner-draw" => self.paint_owner_draw(target, draw)?,
            "text" | "button" => self.paint_text(target, draw)?,
            "edit-field" | "numeric-field" => self.paint_text_field(target, draw)?,
            "list-box" => self.paint_list(target, draw)?,
            "model" => self.paint_model(target, draw)?,
            "yes-no" => self.paint_yes_no(target, draw)?,
            "multi" => self.paint_multi(target, draw)?,
            "bind" => self.paint_bind(target, draw)?,
            "slider" => self.paint_slider(target, draw)?,
            _ => {}
        }
        Ok(())
    }

    /// Advance orbit/transition animation (`advanceItemAnimation`).
    fn advance_item_animation(&mut self, item: ItemState) -> Result<(), ClientError> {
        if self.item(item)?.window().flags() & UiWindowFlag::ORBITING != 0
            && self.real_time > self.item(item)?.window().next_time()
        {
            let real_time = self.real_time;
            let definition = self.menu_item(item.menu_index, item.item_index)?;
            definition
                .window()
                .set_next_time(real_time.wrapping_add(definition.window().offset_time()));
            let half_width = f(definition.window().client_rect().width() / 2.0);
            let half_height = f(definition.window().client_rect().height() / 2.0);
            let rx = f(f(definition.window().client_rect().x() + half_width) - definition.window().rect_effects().x());
            let ry = f(f(definition.window().client_rect().y() + half_height) - definition.window().rect_effects().y());
            // Donor f64 point: f(3 * PI / 180), then f(cos/sin) back to f32.
            let angle = (3.0f64 * std::f64::consts::PI / 180.0) as f32;
            let cosine = f((f64::from(angle)).cos() as f32);
            let sine = f((f64::from(angle)).sin() as f32);
            definition.window().client_rect().set_x(f(f(
                f(f(rx * cosine) - f(ry * sine)) + definition.window().rect_effects().x()
            ) - half_width));
            definition.window().client_rect().set_y(f(f(
                f(f(rx * sine) + f(ry * cosine)) + definition.window().rect_effects().y()
            ) - half_height));
            self.update_item_position(item)?;
        }
        if self.item(item)?.window().flags() & UiWindowFlag::IN_TRANSITION != 0
            && self.real_time > self.item(item)?.window().next_time()
        {
            let real_time = self.real_time;
            let mut done = 0;
            {
                let definition = self.menu_item(item.menu_index, item.item_index)?;
                definition
                    .window()
                    .set_next_time(real_time.wrapping_add(definition.window().offset_time()));
                for component in 0..4 {
                    let current = match component {
                        0 => definition.window().client_rect().x(),
                        1 => definition.window().client_rect().y(),
                        2 => definition.window().client_rect().width(),
                        _ => definition.window().client_rect().height(),
                    };
                    let target = match component {
                        0 => definition.window().rect_effects().x(),
                        1 => definition.window().rect_effects().y(),
                        2 => definition.window().rect_effects().width(),
                        _ => definition.window().rect_effects().height(),
                    };
                    if current == target {
                        done += 1;
                        continue;
                    }
                    let amount = match component {
                        0 => definition.window().rect_effects2().x(),
                        1 => definition.window().rect_effects2().y(),
                        2 => definition.window().rect_effects2().width(),
                        _ => definition.window().rect_effects2().height(),
                    };
                    let (value, finished) = Self::transition_value(current, target, amount);
                    match component {
                        0 => definition.window().client_rect().set_x(value),
                        1 => definition.window().client_rect().set_y(value),
                        2 => definition.window().client_rect().set_width(value),
                        _ => definition.window().client_rect().set_height(value),
                    }
                    if finished {
                        done += 1;
                    }
                }
            }
            self.update_item_position(item)?;
            if done == 4 {
                let window = self.menu_item(item.menu_index, item.item_index)?.window();
                window.set_flags(window.flags() & !UiWindowFlag::IN_TRANSITION);
            }
        }
        Ok(())
    }

    /// Step one transition component (`transitionValue`).
    fn transition_value(current: f32, target: f32, amount: f32) -> (f32, bool) {
        if current == target {
            return (current, true);
        }
        let next = if current < target {
            f(current + amount)
        } else {
            f(current - amount)
        };
        if (current < target && next > target) || (current > target && next < target) {
            (target, true)
        } else {
            (next, false)
        }
    }

    /// Item text or cvar value (`textValue`).
    fn text_value(&self, item: ItemState) -> Result<Option<String>, ClientError> {
        let definition = self.item(item)?;
        if let Some(text) = definition.text().as_deref() {
            return Ok(Some(quake_string(text, None)?));
        }
        match definition.cvar().as_deref() {
            Some(cvar) => Ok(Some(self.cvar_buffer(cvar)?)),
            None => Ok(None),
        }
    }

    /// Measure and place item text (`textExtents`).
    fn text_extents(&mut self, item: ItemState, text: &str) -> Result<(), ClientError> {
        let (width, kind, alignment) = {
            let definition = self.item(item)?;
            (
                definition.text_rect().width(),
                definition.behavior().kind(),
                definition.text_alignment(),
            )
        };
        if qvm_float_to_int(width) != 0 && !(kind == "owner-draw" && alignment == 1) {
            return Ok(());
        }
        let (static_text, scale, owner_draw, cvar) = {
            let definition = self.item(item)?;
            (
                definition.text().unwrap_or_default(),
                definition.text_scale(),
                definition.window().owner_draw(),
                definition.cvar(),
            )
        };
        let mut original = text_width(&self.options.fonts, &static_text, scale, 0)?;
        if kind == "owner-draw" && (alignment == 1 || alignment == 2) {
            original += self.options.owner_draw.width(owner_draw, scale);
        } else if kind == "edit-field" && alignment == 1 {
            if let Some(cvar) = cvar.as_deref() {
                let buffer = self.cvar_buffer(cvar)?;
                let trimmed: String = buffer.chars().take(255).collect();
                original += text_width(&self.options.fonts, &trimmed, scale, 0)?;
            }
        }
        let width = text_width(&self.options.fonts, text, scale, 0)?;
        let height = text_height(&self.options.fonts, text, scale, 0)?;
        let (align_x, align_y, border, border_size, rect) = {
            let definition = self.item(item)?;
            (
                definition.text_align_x(),
                definition.text_align_y(),
                definition.window().border(),
                definition.window().border_size(),
                definition.window().rect(),
            )
        };
        let mut placed = UiRect {
            x: align_x,
            y: align_y,
            width: width as f32,
            height: height as f32,
        };
        if alignment == 2 {
            placed.x = f(align_x - original as f32);
        } else if alignment == 1 {
            placed.x = f(align_x - (original / 2) as f32);
        }
        if border != 0 {
            placed.x = f(placed.x + border_size);
            placed.y = f(placed.y + border_size);
        }
        placed.x = f(placed.x + rect.x());
        placed.y = f(placed.y + rect.y());
        self.menu_item(item.menu_index, item.item_index)?.set_text_rect(&placed);
        Ok(())
    }

    /// Item text color with fade/focus/blink (`itemTextColor`).
    fn item_text_color(&mut self, item: ItemState) -> Result<Vec4, ClientError> {
        let menu = self.menu_for(item)?;
        let (fade_clamp, fade_cycle, fade_amount, focus_color, disable_color) = {
            let menu_def = self.menu(menu);
            (
                menu_def.fade_clamp(),
                menu_def.fade_cycle(),
                menu_def.fade_amount(),
                menu_def.focus_color().snapshot(),
                menu_def.disable_color().snapshot(),
            )
        };
        let real_time = self.real_time;
        Self::fade_window(
            real_time,
            &mut self.menu_item(item.menu_index, item.item_index)?.window(),
            true,
            fade_clamp,
            fade_cycle,
            true,
            fade_amount,
        );
        let (mut color, fore, style, flags) = {
            let definition = self.item(item)?;
            (
                definition.window().fore_color().snapshot(),
                definition.window().fore_color().snapshot(),
                definition.text_style(),
                definition.window().flags(),
            )
        };
        if flags & UiWindowFlag::HAS_FOCUS != 0 {
            color = self.pulse(focus_color);
        } else if style == 1 && (self.real_time / BLINK_DIVISOR) & 1 == 0 {
            color = self.pulse(fore);
        }
        if !self.item_passes_cvar(item, "enable")? {
            return Ok(disable_color);
        }
        Ok(color)
    }

    /// Focus pulse color (`pulse`).
    fn pulse(&self, color: Vec4) -> Vec4 {
        self.lerp_color(
            color,
            Vec4 {
                x: f(0.8 * color.x),
                y: f(0.8 * color.y),
                z: f(0.8 * color.z),
                w: f(0.8 * color.w),
            },
        )
    }

    /// Pulsed color interpolation (`lerpColor`).
    fn lerp_color(&self, color: Vec4, low_light: Vec4) -> Vec4 {
        // Donor f64 point: Math.sin over the truncated step, rounded to f32.
        let sine = f(f64::from(f((self.real_time / PULSE_DIVISOR) as f32)).sin() as f32);
        let amount = f(0.5 + f(0.5 * sine));
        let component = |from: f32, to: f32| -> f32 { (f(from + f(amount * f(to - from)))).clamp(0.0, 1.0) };
        Vec4 {
            x: component(color.x, low_light.x),
            y: component(color.y, low_light.y),
            z: component(color.z, low_light.z),
            w: component(color.w, low_light.w),
        }
    }

    /// Paint item text (`paintText`).
    fn paint_text(&mut self, item: ItemState, draw: &mut Draw2D) -> Result<(), ClientError> {
        let value = self.text_value(item)?;
        let Some(text) = value else {
            return Ok(());
        };
        let flags = self.item(item)?.window().flags();
        let wrapped = flags & (UiWindowFlag::WRAPPED | UiWindowFlag::AUTO_WRAPPED) != 0;
        let color = if wrapped {
            if text.is_empty() {
                return Ok(());
            }
            let color = self.item_text_color(item)?;
            self.text_extents(item, &text)?;
            color
        } else {
            self.text_extents(item, &text)?;
            if text.is_empty() {
                return Ok(());
            }
            self.item_text_color(item)?
        };
        if flags & UiWindowFlag::WRAPPED != 0 {
            let (mut y, height, x, scale, style) = {
                let definition = self.item(item)?;
                (
                    definition.text_rect().y(),
                    definition.text_rect().height(),
                    definition.text_rect().x(),
                    definition.text_scale(),
                    definition.text_style(),
                )
            };
            let lines: Vec<&str> = text.split('\r').collect();
            for (index, line) in lines.iter().enumerate() {
                if index + 1 < lines.len() && line.len() >= EDIT_BUFFER_LEN {
                    return Err(bad_ui("Item_Text_Wrapped_Paint writes outside local char[1024]"));
                }
                text_paint(
                    draw,
                    &self.options.fonts,
                    &TextPaintOptions {
                        x,
                        y,
                        scale,
                        color,
                        text: line,
                        adjust: 0.0,
                        limit: 0,
                        style,
                    },
                )?;
                y = f(y + f((qvm_float_to_int(height) + 5) as f32));
            }
            return Ok(());
        }
        if flags & UiWindowFlag::AUTO_WRAPPED != 0 {
            self.paint_auto_wrapped(item, draw, &text, color)?;
            return Ok(());
        }
        let (x, y, scale, style) = {
            let definition = self.item(item)?;
            (
                definition.text_rect().x(),
                definition.text_rect().y(),
                definition.text_scale(),
                definition.text_style(),
            )
        };
        text_paint(
            draw,
            &self.options.fonts,
            &TextPaintOptions {
                x,
                y,
                scale,
                color,
                text: &text,
                adjust: 0.0,
                limit: 0,
                style,
            },
        )?;
        Ok(())
    }

    /// Paint auto-wrapped text (`paintAutoWrapped`).
    fn paint_auto_wrapped(
        &mut self,
        item: ItemState,
        draw: &mut Draw2D,
        text: &str,
        color: Vec4,
    ) -> Result<(), ClientError> {
        let chars: Vec<char> = text.chars().collect();
        let mut buffer = String::new();
        let mut length = 0usize;
        let mut position = 0usize;
        let mut width = 0i32;
        let mut line_break = 0usize;
        let mut next_line = 0usize;
        let mut line_width = 0i32;
        let (mut y, scale, style, alignment, align_x) = {
            let definition = self.item(item)?;
            (
                definition.text_align_y(),
                definition.text_scale(),
                definition.text_style(),
                definition.text_alignment(),
                definition.text_align_x(),
            )
        };
        loop {
            let character = chars.get(position).copied().unwrap_or('\0');
            let is_break = character == ' ' || character == '\t' || character == '\n' || character == '\0';
            if is_break {
                line_break = length;
                next_line = position + 1;
                line_width = width;
            }
            width = text_width(&self.options.fonts, &buffer, scale, 0)?;
            let rect_width = self.item(item)?.window().rect().width();
            if (line_break != 0 && width as f32 > rect_width) || character == '\n' || character == '\0' {
                if length != 0 {
                    let mut x = self.item(item)?.text_rect().x();
                    if alignment == 0 {
                        x = align_x;
                    } else if alignment == 2 {
                        x = f(align_x - line_width as f32);
                    } else if alignment == 1 {
                        x = f(align_x - (line_width / 2) as f32);
                    }
                    let mut baseline = y;
                    let (border, border_size, rect) = {
                        let definition = self.item(item)?;
                        (
                            definition.window().border(),
                            definition.window().border_size(),
                            definition.window().rect(),
                        )
                    };
                    if border != 0 {
                        x = f(x + border_size);
                        baseline = f(baseline + border_size);
                    }
                    self.menu_item(item.menu_index, item.item_index)?
                        .text_rect()
                        .set_x(f(x + rect.x()));
                    self.menu_item(item.menu_index, item.item_index)?
                        .text_rect()
                        .set_y(f(baseline + rect.y()));
                    buffer.truncate(line_break);
                    let (paint_x, paint_y) = {
                        let definition = self.item(item)?;
                        (definition.text_rect().x(), definition.text_rect().y())
                    };
                    text_paint(
                        draw,
                        &self.options.fonts,
                        &TextPaintOptions {
                            x: paint_x,
                            y: paint_y,
                            scale,
                            color,
                            text: &buffer,
                            adjust: 0.0,
                            limit: 0,
                            style,
                        },
                    )?;
                }
                if character == '\0' {
                    return Ok(());
                }
                let height = self.item(item)?.text_rect().height();
                y = f(y + f((qvm_float_to_int(height) + 5) as f32));
                position = next_line;
                length = 0;
                line_break = 0;
                line_width = 0;
            } else {
                if length >= MAX_SCRIPT_BYTES {
                    return Err(bad_ui("Item_Text_AutoWrapped_Paint writes outside local char[1024]"));
                }
                buffer.truncate(length);
                buffer.push(character);
                length += 1;
                position += 1;
            }
        }
    }

    /// Value text color (`valueColor`).
    fn value_color(&self, item: ItemState) -> Result<Vec4, ClientError> {
        if self.item(item)?.window().flags() & UiWindowFlag::HAS_FOCUS != 0 {
            let menu = self.menu_for(item)?;
            Ok(self.pulse(self.menu(menu).focus_color().snapshot()))
        } else {
            Ok(self.item(item)?.window().fore_color().snapshot())
        }
    }

    /// Paint a text field (`paintTextField`).
    fn paint_text_field(&mut self, item: ItemState, draw: &mut Draw2D) -> Result<(), ClientError> {
        let edit = self.item(item)?.edit_data();
        self.paint_text(item, draw)?;
        let cvar = self.item(item)?.cvar();
        let value = match cvar.as_deref() {
            Some(cvar) => self.cvar_buffer(cvar)?,
            None => String::new(),
        };
        let color = self.value_color(item)?;
        let has_text = self.item(item)?.text().as_ref().is_some_and(|text| !text.is_empty());
        let offset = if has_text { 8.0 } else { 0.0 };
        let Some(edit) = edit else {
            return Err(bad_ui("Item_TextField_Paint dereferences NULL edit data"));
        };
        let (x, y, width, scale, style, cursor_position) = {
            let definition = self.item(item)?;
            (
                definition.text_rect().x(),
                definition.text_rect().y(),
                definition.text_rect().width(),
                definition.text_scale(),
                definition.text_style(),
                definition.cursor_position(),
            )
        };
        let visible: String = value.chars().skip(edit.paint_offset().max(0) as usize).collect();
        let options = TextPaintOptions {
            x: f(f(x + width) + offset),
            y,
            scale,
            color,
            text: &visible,
            adjust: 0.0,
            limit: edit.max_paint_chars(),
            style,
        };
        if self.item(item)?.window().flags() & UiWindowFlag::HAS_FOCUS != 0 && self.editing_item.is_some() {
            let overstrike = self.binding_host("paint edit cursor")?.get_overstrike();
            text_paint_with_cursor(
                draw,
                &self.options.fonts,
                &options,
                TextCursor {
                    position: (cursor_position - edit.paint_offset()).max(0) as usize,
                    character: if overstrike { 95 } else { 124 },
                    time: self.real_time,
                },
            )?;
        } else {
            text_paint(draw, &self.options.fonts, &options)?;
        }
        Ok(())
    }

    /// Paint a yes/no toggle (`paintYesNo`).
    fn paint_yes_no(&mut self, item: ItemState, draw: &mut Draw2D) -> Result<(), ClientError> {
        let cvar = self.item(item)?.cvar();
        let value = match cvar.as_deref() {
            Some(cvar) => self.cvar_value(cvar),
            None => 0.0,
        };
        let color = self.value_color(item)?;
        let mut x = self.item(item)?.text_rect().x();
        if self.item(item)?.text().is_some() {
            self.paint_text(item, draw)?;
            let definition = self.item(item)?;
            x = f(f(definition.text_rect().x() + definition.text_rect().width()) + 8.0);
        }
        let (y, scale, style) = {
            let definition = self.item(item)?;
            (
                definition.text_rect().y(),
                definition.text_scale(),
                definition.text_style(),
            )
        };
        text_paint(
            draw,
            &self.options.fonts,
            &TextPaintOptions {
                x,
                y,
                scale,
                color,
                text: if value != 0.0 { "Yes" } else { "No" },
                adjust: 0.0,
                limit: 0,
                style,
            },
        )?;
        Ok(())
    }

    /// Current multi setting label (`multiSetting`).
    fn multi_setting(&mut self, item: ItemState) -> Result<String, ClientError> {
        let (cvar, multi) = {
            let definition = self.item(item)?;
            (
                definition.cvar(),
                match &definition.behavior() {
                    UiItemBehavior::Multi { multi } => multi.clone(),
                    _ => None,
                },
            )
        };
        let (Some(cvar), Some(multi)) = (cvar, multi) else {
            return Ok(String::new());
        };
        if multi.string_definition() {
            let current = self.cvar_buffer(&cvar)?;
            for index in 0..multi.count().max(0) as usize {
                if equal_name(multi.string_value(index).as_deref(), &current) {
                    return Ok(multi.label(index).unwrap_or_default().to_string());
                }
            }
        } else {
            let current = self.cvar_value(&cvar);
            for index in 0..multi.count().max(0) as usize {
                if multi.number_value(index) == current {
                    return Ok(multi.label(index).unwrap_or_default().to_string());
                }
            }
        }
        Ok(String::new())
    }

    /// Paint a multi-choice (`paintMulti`).
    fn paint_multi(&mut self, item: ItemState, draw: &mut Draw2D) -> Result<(), ClientError> {
        let color = self.value_color(item)?;
        let text = self.multi_setting(item)?;
        let mut x = self.item(item)?.text_rect().x();
        if self.item(item)?.text().is_some() {
            self.paint_text(item, draw)?;
            let definition = self.item(item)?;
            x = f(f(definition.text_rect().x() + definition.text_rect().width()) + 8.0);
        }
        let (y, scale, style) = {
            let definition = self.item(item)?;
            (
                definition.text_rect().y(),
                definition.text_scale(),
                definition.text_style(),
            )
        };
        text_paint(
            draw,
            &self.options.fonts,
            &TextPaintOptions {
                x,
                y,
                scale,
                color,
                text: &text,
                adjust: 0.0,
                limit: 0,
                style,
            },
        )?;
        Ok(())
    }

    /// Binding display text (`bindingText`).
    fn binding_text(&mut self, item: ItemState) -> Result<String, ClientError> {
        let cvar = self.item(item)?.cvar();
        let binding = self.binding_by_name(cvar.as_deref());
        let Some(binding) = binding else {
            return Ok("???".to_string());
        };
        if self.bindings[binding].first == NO_BINDING {
            return Ok("???".to_string());
        }
        let (first_key, second_key) = (self.bindings[binding].first, self.bindings[binding].second);
        let first = self.binding_host("paint binding")?.key_name(first_key);
        let first: String = quake_string(&first, None)?.to_uppercase().chars().take(31).collect();
        if second_key == NO_BINDING {
            return Ok(first);
        }
        let second = self.binding_host("paint binding")?.key_name(second_key);
        let second: String = quake_string(&second, None)?.to_uppercase().chars().take(31).collect();
        Ok(format!("{first} or {second}").chars().take(31).collect())
    }

    /// Paint a binding (`paintBind`).
    fn paint_bind(&mut self, item: ItemState, draw: &mut Draw2D) -> Result<(), ClientError> {
        let max_chars = self
            .item(item)?
            .edit_data()
            .map(|edit| edit.max_paint_chars())
            .unwrap_or(0);
        let cvar = self.item(item)?.cvar();
        if let Some(cvar) = cvar.as_deref() {
            let _ = self.cvar_value(cvar);
        }
        let mut color = self.value_color(item)?;
        if self.item(item)?.window().flags() & UiWindowFlag::HAS_FOCUS != 0 && self.binding_item == Some(item) {
            let menu = self.menu_for(item)?;
            let focus = self.menu(menu).focus_color().snapshot();
            color = self.lerp_color(
                focus,
                Vec4 {
                    x: f(0.8),
                    y: 0.0,
                    z: 0.0,
                    w: f(0.8),
                },
            );
        }
        let mut x = self.item(item)?.text_rect().x();
        let mut text = "FIXME".to_string();
        if self.item(item)?.text().is_some() {
            self.paint_text(item, draw)?;
            text = self.binding_text(item)?;
            let definition = self.item(item)?;
            x = f(f(definition.text_rect().x() + definition.text_rect().width()) + 8.0);
        }
        let (y, scale, style) = {
            let definition = self.item(item)?;
            (
                definition.text_rect().y(),
                definition.text_scale(),
                definition.text_style(),
            )
        };
        text_paint(
            draw,
            &self.options.fonts,
            &TextPaintOptions {
                x,
                y,
                scale,
                color,
                text: &text,
                adjust: 0.0,
                limit: max_chars,
                style,
            },
        )?;
        Ok(())
    }

    /// Paint a slider (`paintSlider`).
    fn paint_slider(&mut self, item: ItemState, draw: &mut Draw2D) -> Result<(), ClientError> {
        let cvar = self.item(item)?.cvar();
        if let Some(cvar) = cvar.as_deref() {
            let _ = self.cvar_value(cvar);
        }
        let color = self.value_color(item)?;
        let y = self.item(item)?.window().rect().y();
        let mut x;
        if self.item(item)?.text().is_some() {
            self.paint_text(item, draw)?;
            let definition = self.item(item)?;
            x = f(f(definition.text_rect().x() + definition.text_rect().width()) + 8.0);
        } else {
            x = self.item(item)?.window().rect().x();
        }
        draw.set_color(Some(color));
        let bar = self.widget_picture(self.options.widget_assets.slider_bar)?;
        draw.draw_handle_pic(
            Rect {
                x,
                y,
                width: SLIDER_WIDTH,
                height: 16.0,
            },
            bar,
        );
        x = self.slider_thumb_position(item)?;
        let thumb = self.widget_picture(self.options.widget_assets.slider_thumb)?;
        draw.draw_handle_pic(
            Rect {
                x: f(x - SLIDER_THUMB_WIDTH / 2.0),
                y: f(y - 2.0),
                width: SLIDER_THUMB_WIDTH,
                height: 20.0,
            },
            thumb,
        );
        Ok(())
    }

    /// Paint a model (`paintModel`).
    fn paint_model(&mut self, item: ItemState, draw: &mut Draw2D) -> Result<(), ClientError> {
        let model_data = self.item(item)?.model_data();
        let Some(model_data) = model_data else {
            return Ok(());
        };
        if model_data.rotation_speed() != 0 && self.real_time > self.item(item)?.window().next_time() {
            let real_time = self.real_time;
            let definition = self.item(item)?;
            definition
                .window()
                .set_next_time(real_time.wrapping_add(model_data.rotation_speed()));
            model_data.set_angle((model_data.angle() + 1) % 360);
        }
        let (handle, asset) = {
            let definition = self.item(item)?;
            (definition.asset_handle(), definition.asset())
        };
        let asset_path = match asset.as_ref() {
            Some(UiMenuResource::Shader(shader)) => shader.path.clone(),
            Some(UiMenuResource::Model(model)) => model.path.clone(),
            Some(UiMenuResource::Sound(_)) => None,
            None => None,
        };
        let model = match handle {
            None => match asset.as_ref() {
                Some(UiMenuResource::Model(model)) => self.models.get(&resource_key(model.path.as_deref())).cloned(),
                _ => None,
            },
            Some(0) if self.options.resources.handle_kind() == UiHandleKind::Diagnostic => Some(default_model()),
            Some(handle) => Some(self.options.resources.model_for_handle(handle)?),
        };
        let Some(model) = model else {
            return Err(bad_ui(format!(
                "UI model {} was not resolved before runtime use",
                asset_path.as_deref().unwrap_or("NULL")
            )));
        };
        let rect = self.item(item)?.window().rect();
        let time = self.real_time;
        (self.options.paint_model)(UiModelPaintRequest {
            draw,
            model: &model,
            rect: UiRect {
                x: f(rect.x() + 1.0),
                y: f(rect.y() + 1.0),
                width: f(rect.width() - 2.0),
                height: f(rect.height() - 2.0),
            },
            time,
            angle: model_data.angle(),
            field_of_view_x: model_data.field_of_view_x(),
            field_of_view_y: model_data.field_of_view_y(),
        });
        Ok(())
    }

    /// Paint an owner-draw item (`paintOwnerDraw`).
    ///
    /// `Promise -> sync`.
    fn paint_owner_draw(&mut self, item: ItemState, draw: &mut Draw2D) -> Result<(), ClientError> {
        let menu = self.menu_for(item)?;
        let (fade_clamp, fade_cycle, fade_amount, focus_color, disable_color) = {
            let menu_def = self.menu(menu);
            (
                menu_def.fade_clamp(),
                menu_def.fade_cycle(),
                menu_def.fade_amount(),
                menu_def.focus_color().snapshot(),
                menu_def.disable_color().snapshot(),
            )
        };
        let real_time = self.real_time;
        Self::fade_window(
            real_time,
            &mut self.menu_item(item.menu_index, item.item_index)?.window(),
            true,
            fade_clamp,
            fade_cycle,
            true,
            fade_amount,
        );
        let mut color = self.item(item)?.window().fore_color().snapshot();
        let ranges = self.item(item)?.color_ranges()?;
        if !ranges.is_empty() {
            let owner_draw = self.item(item)?.window().owner_draw();
            let value = self.options.owner_draw.value(owner_draw);
            for range in &ranges {
                if value >= range.low && value <= range.high {
                    color = range.color;
                    break;
                }
            }
        }
        let (flags, style, fore) = {
            let definition = self.item(item)?;
            (
                definition.window().flags(),
                definition.text_style(),
                definition.window().fore_color().snapshot(),
            )
        };
        if flags & UiWindowFlag::HAS_FOCUS != 0 {
            color = self.pulse(focus_color);
        } else if style == 1 && (self.real_time / BLINK_DIVISOR) & 1 == 0 {
            color = self.pulse(fore);
        }
        if !self.item_passes_cvar(item, "enable")? {
            color = disable_color;
        }
        let rect = self.item(item)?.window().rect();
        let mut text_x = self.item(item)?.text_align_x();
        let has_text = self.item(item)?.text().is_some();
        let text_len = self.item(item)?.text().as_ref().is_some_and(|text| !text.is_empty());
        if has_text {
            self.paint_text(item, draw)?;
            let definition = self.item(item)?;
            rect.set_x(f(
                f(definition.text_rect().x() + definition.text_rect().width()) + if text_len { 8.0 } else { 0.0 }
            ));
            text_x = 0.0;
        }
        let definition = self.item(item)?;
        let background = self.window_background(&definition.window())?;
        self.options.owner_draw.paint(UiOwnerDrawPaintRequest {
            draw,
            rect: rect.snapshot(),
            text_x,
            text_y: definition.text_align_y(),
            owner_draw: definition.window().owner_draw(),
            owner_draw_flags: definition.window().owner_draw_flags(),
            alignment: definition.alignment(),
            special: definition.special(),
            text_scale: definition.text_scale(),
            color,
            background,
            text_style: definition.text_style(),
        });
        Ok(())
    }

    /// Paint a list box (`paintList`).
    ///
    /// `Promise -> sync`.
    fn paint_list(&mut self, item: ItemState, draw: &mut Draw2D) -> Result<(), ClientError> {
        let list = self.item(item)?.list_data();
        let special = self.item(item)?.special();
        let count = f(self.options.feeder.count(special) as f32);
        let horizontal = self.item(item)?.window().flags() & UiWindowFlag::HORIZONTAL != 0;
        let rect = self.item(item)?.window().rect();
        if horizontal {
            let mut x = f(rect.x() + 1.0);
            let y = f(f(f(rect.y() + rect.height()) - SCROLLBAR_SIZE) - 1.0);
            draw.draw_handle_pic(
                Rect {
                    x,
                    y,
                    width: SCROLLBAR_SIZE,
                    height: SCROLLBAR_SIZE,
                },
                self.options.widget_assets.scroll_bar_arrow_left,
            );
            x = f(x + SCROLLBAR_SIZE - 1.0);
            let mut size = f(rect.width() - SCROLLBAR_SIZE * 2.0);
            draw.draw_handle_pic(
                Rect {
                    x,
                    y,
                    width: f(size + 1.0),
                    height: SCROLLBAR_SIZE,
                },
                self.options.widget_assets.scroll_bar,
            );
            x = f(x + f(size - 1.0));
            draw.draw_handle_pic(
                Rect {
                    x,
                    y,
                    width: SCROLLBAR_SIZE,
                    height: SCROLLBAR_SIZE,
                },
                self.options.widget_assets.scroll_bar_arrow_right,
            );
            let thumb = f(self.list_thumb_draw_position(item)? as f32).min(f(f(x - SCROLLBAR_SIZE) - 1.0));
            draw.draw_handle_pic(
                Rect {
                    x: thumb,
                    y,
                    width: SCROLLBAR_SIZE,
                    height: SCROLLBAR_SIZE,
                },
                self.options.widget_assets.scroll_bar_thumb,
            );
            let Some(list) = list else {
                return Err(bad_ui("Item_ListBox_Paint dereferences NULL list data"));
            };
            self.write_list_start(item, list.start_position())?;
            size = f(rect.width() - 2.0);
            if list.element_style() != 1 {
                return Ok(());
            }
            let mut x = f(rect.x() + 1.0);
            let y = f(rect.y() + 1.0);
            let mut row = f(list.start_position() as f32);
            while row < count {
                let picture = self.options.feeder.image(special, qvm_float_to_int(row));
                if let Some(picture) = picture {
                    draw.draw_handle_pic(
                        Rect {
                            x: f(x + 1.0),
                            y: f(y + 1.0),
                            width: f(list.element_width() - 2.0),
                            height: f(list.element_height() - 2.0),
                        },
                        picture,
                    );
                }
                if row == f(self.item(item)?.cursor_position() as f32) {
                    let (border_size, border_color) = {
                        let definition = self.item(item)?;
                        (
                            definition.window().border_size(),
                            definition.window().border_color().snapshot(),
                        )
                    };
                    self.draw_rect(
                        draw,
                        &UiRect {
                            x,
                            y,
                            width: f(list.element_width() - 1.0),
                            height: f(list.element_height() - 1.0),
                        },
                        border_size,
                        border_color,
                    );
                }
                size = f(size - list.element_width());
                if size < list.element_width() {
                    self.write_list_padding(item, qvm_float_to_int(size))?;
                    break;
                }
                x = f(x + list.element_width());
                self.bump_list_end(item)?;
                row = f(row + 1.0);
            }
            return Ok(());
        }
        let x = f(f(f(rect.x() + rect.width()) - SCROLLBAR_SIZE) - 1.0);
        let mut y = f(rect.y() + 1.0);
        draw.draw_handle_pic(
            Rect {
                x,
                y,
                width: SCROLLBAR_SIZE,
                height: SCROLLBAR_SIZE,
            },
            self.options.widget_assets.scroll_bar_arrow_up,
        );
        y = f(y + SCROLLBAR_SIZE - 1.0);
        let Some(list) = list else {
            return Err(bad_ui("Item_ListBox_Paint dereferences NULL list data"));
        };
        self.write_list_start(item, list.start_position())?;
        let mut size = f(rect.height() - SCROLLBAR_SIZE * 2.0);
        draw.draw_handle_pic(
            Rect {
                x,
                y,
                width: SCROLLBAR_SIZE,
                height: f(size + 1.0),
            },
            self.options.widget_assets.scroll_bar,
        );
        y = f(y + f(size - 1.0));
        draw.draw_handle_pic(
            Rect {
                x,
                y,
                width: SCROLLBAR_SIZE,
                height: SCROLLBAR_SIZE,
            },
            self.options.widget_assets.scroll_bar_arrow_down,
        );
        let thumb = f(self.list_thumb_draw_position(item)? as f32).min(f(f(y - SCROLLBAR_SIZE) - 1.0));
        draw.draw_handle_pic(
            Rect {
                x,
                y: thumb,
                width: SCROLLBAR_SIZE,
                height: SCROLLBAR_SIZE,
            },
            self.options.widget_assets.scroll_bar_thumb,
        );
        size = f(rect.height() - 2.0);
        let x = f(rect.x() + 1.0);
        let mut y = f(rect.y() + 1.0);
        let image_style = list.element_style() == 1;
        let mut row = f(list.start_position() as f32);
        while row < count {
            if image_style {
                let picture = self.options.feeder.image(special, qvm_float_to_int(row));
                if let Some(picture) = picture {
                    draw.draw_handle_pic(
                        Rect {
                            x: f(x + 1.0),
                            y: f(y + 1.0),
                            width: f(list.element_width() - 2.0),
                            height: f(list.element_height() - 2.0),
                        },
                        picture,
                    );
                }
                if row == f(self.item(item)?.cursor_position() as f32) {
                    let (border_size, border_color) = {
                        let definition = self.item(item)?;
                        (
                            definition.window().border_size(),
                            definition.window().border_color().snapshot(),
                        )
                    };
                    self.draw_rect(
                        draw,
                        &UiRect {
                            x,
                            y,
                            width: f(list.element_width() - 1.0),
                            height: f(list.element_height() - 1.0),
                        },
                        border_size,
                        border_color,
                    );
                }
                self.bump_list_end(item)?;
                size = f(size - list.element_width());
            } else if list.column_count() > 0 {
                for column_index in 0..list.column_count().max(0) as usize {
                    let Some(column) = list.column_at(column_index) else {
                        continue;
                    };
                    let entry = self
                        .options
                        .feeder
                        .item(special, qvm_float_to_int(row), column_index as i32);
                    let Some(entry) = entry else {
                        continue;
                    };
                    if let Some(picture) = entry.picture {
                        draw.draw_handle_pic(
                            Rect {
                                x: f(f(x + 4.0) + f(column.position as f32)),
                                y: f(f(y - 1.0) + f(list.element_height() / 2.0)),
                                width: f(column.width as f32),
                                height: f(column.width as f32),
                            },
                            picture,
                        );
                    } else if let Some(text) = entry.text.as_deref() {
                        let (fore, scale, style) = {
                            let definition = self.item(item)?;
                            (
                                definition.window().fore_color(),
                                definition.text_scale(),
                                definition.text_style(),
                            )
                        };
                        let text = quake_string(text, None)?;
                        text_paint(
                            draw,
                            &self.options.fonts,
                            &TextPaintOptions {
                                x: f(f(x + 4.0) + f(column.position as f32)),
                                y: f(y + list.element_height()),
                                scale,
                                color: fore.snapshot(),
                                text: &text,
                                adjust: 0.0,
                                limit: column.max_chars,
                                style,
                            },
                        )?;
                    }
                }
                if row == f(self.item(item)?.cursor_position() as f32) {
                    let (outline, rect) = {
                        let definition = self.item(item)?;
                        (
                            definition.window().outline_color().snapshot(),
                            definition.window().rect(),
                        )
                    };
                    draw.fill_rect(
                        Rect {
                            x: f(x + 2.0),
                            y: f(y + 2.0),
                            width: f(f(rect.width() - SCROLLBAR_SIZE) - 4.0),
                            height: list.element_height(),
                        },
                        outline,
                        self.options.widget_assets.white_shader,
                    );
                }
                size = f(size - list.element_height());
            } else {
                let entry = self.options.feeder.item(special, qvm_float_to_int(row), 0);
                if let Some(entry) = entry {
                    if entry.picture.is_none() {
                        if let Some(text) = entry.text.as_deref() {
                            let (fore, scale, style) = {
                                let definition = self.item(item)?;
                                (
                                    definition.window().fore_color(),
                                    definition.text_scale(),
                                    definition.text_style(),
                                )
                            };
                            let text = quake_string(text, None)?;
                            text_paint(
                                draw,
                                &self.options.fonts,
                                &TextPaintOptions {
                                    x: f(x + 4.0),
                                    y: f(y + list.element_height()),
                                    scale,
                                    color: fore.snapshot(),
                                    text: &text,
                                    adjust: 0.0,
                                    limit: 0,
                                    style,
                                },
                            )?;
                        }
                    }
                }
                if row == f(self.item(item)?.cursor_position() as f32) {
                    let (outline, rect) = {
                        let definition = self.item(item)?;
                        (
                            definition.window().outline_color().snapshot(),
                            definition.window().rect(),
                        )
                    };
                    draw.fill_rect(
                        Rect {
                            x: f(x + 2.0),
                            y: f(y + 2.0),
                            width: f(f(rect.width() - SCROLLBAR_SIZE) - 4.0),
                            height: list.element_height(),
                        },
                        outline,
                        self.options.widget_assets.white_shader,
                    );
                }
                size = f(size - list.element_height());
            }
            if size < list.element_height() {
                self.write_list_padding(item, qvm_float_to_int(f(list.element_height() - size)))?;
                break;
            }
            if !image_style {
                self.bump_list_end(item)?;
            }
            y = f(y + list.element_height());
            row = f(row + 1.0);
        }
        Ok(())
    }

    /// Reset a list's painted end to its start.
    fn write_list_start(&mut self, item: ItemState, start: i32) -> Result<(), ClientError> {
        if let Some(list) = self.item(item)?.list_data() {
            list.set_end_position(start);
        }
        Ok(())
    }

    /// Increment a list's painted end.
    fn bump_list_end(&mut self, item: ItemState) -> Result<(), ClientError> {
        if let Some(list) = self.item(item)?.list_data() {
            list.set_end_position(list.end_position() + 1);
        }
        Ok(())
    }

    /// Write a list's draw padding.
    fn write_list_padding(&mut self, item: ItemState, padding: i32) -> Result<(), ClientError> {
        if let Some(list) = self.item(item)?.list_data() {
            list.set_draw_padding(padding);
        }
        Ok(())
    }

    /// Item behavior snapshot (`behaviorSnapshot`).
    fn behavior_snapshot(&self, item: ItemState) -> Result<UiRuntimeItemBehaviorSnapshot, ClientError> {
        let definition = self.item(item)?;
        if let UiItemBehavior::ListBox { list } = &definition.behavior() {
            return Ok(UiRuntimeItemBehaviorSnapshot::ListBox {
                start_position: list.start_position(),
                end_position: list.end_position(),
                cursor_position: list.cursor_position(),
                draw_padding: list.draw_padding(),
            });
        }
        if matches!(
            definition.behavior().kind(),
            "edit-field" | "numeric-field" | "slider" | "yes-no" | "bind" | "text"
        ) {
            if let Some(edit) = definition.edit_data() {
                return Ok(UiRuntimeItemBehaviorSnapshot::Edit {
                    paint_offset: edit.paint_offset(),
                });
            }
        }
        Ok(UiRuntimeItemBehaviorSnapshot::Other)
    }

    /// Runtime snapshot (`snapshot`).
    pub fn snapshot(&self) -> Result<UiRuntimeSnapshot, ClientError> {
        self.opened()?;
        let focused = self.focused_menu();
        let mut menus = Vec::new();
        for menu in 0..self.active_menu_count {
            let mut items = Vec::new();
            for item in 0..self.definitions.menus[menu].item_count().max(0) as usize {
                let target = ItemState {
                    menu_index: menu,
                    item_index: item,
                };
                let definition = self.item(target)?;
                items.push(UiRuntimeItemSnapshot {
                    name: definition.window().name(),
                    group: definition.window().group(),
                    flags: definition.window().flags(),
                    rect: definition.window().rect().snapshot(),
                    client_rect: definition.window().client_rect().snapshot(),
                    fore_color: definition.window().fore_color().snapshot(),
                    back_color: definition.window().back_color().snapshot(),
                    border_color: definition.window().border_color().snapshot(),
                    background: self.window_picture(&definition.window())?,
                    cursor_position: definition.cursor_position(),
                    special: definition.special(),
                    enabled: self.item_passes_cvar(target, "enable")?,
                    shown: self.item_passes_cvar(target, "show")?,
                    behavior: self.behavior_snapshot(target)?,
                });
            }
            let definition = self.menu(menu);
            menus.push(UiRuntimeMenuSnapshot {
                name: definition.window().name(),
                flags: definition.window().flags(),
                rect: definition.window().rect().snapshot(),
                cursor_item: definition.cursor_item(),
                items,
            });
        }
        Ok(UiRuntimeSnapshot {
            focused_menu: focused.and_then(|index| self.menu(index).window().name()),
            open_stack: self
                .open_stack
                .iter()
                .map(|index| self.definitions.menus[*index].window().name())
                .collect(),
            menus,
        })
    }

    /// Dispose the runtime (`dispose`).
    pub fn dispose(&mut self) {
        if self.disposed {
            return;
        }
        self.close_cinematics();
        self.retire();
    }

    /// Managed release without engine traps (`retire`).
    pub fn retire(&mut self) {
        self.disposed = true;
        self.capture = CaptureState::Idle;
        self.editing_item = None;
        self.binding_item = None;
        self.waiting_for_key = false;
    }
}

/// Parse four script tokens as a rectangle (`parsedRect`).
fn parsed_rect(cursor: &mut RuntimeScriptCursor) -> Result<Option<UiRect>, ClientError> {
    let x = cursor.raw_string();
    let Some(x) = x else {
        return Ok(None);
    };
    let y = cursor.raw_string();
    let Some(y) = y else {
        return Ok(None);
    };
    let width = cursor.raw_string();
    let Some(width) = width else {
        return Ok(None);
    };
    let height = cursor.raw_string();
    let Some(height) = height else {
        return Ok(None);
    };
    Ok(Some(UiRect {
        x: game_atof(&x)?,
        y: game_atof(&y)?,
        width: game_atof(&width)?,
        height: game_atof(&height)?,
    }))
}

/// Transition step for one component (`scriptTransition` step).
fn transition_step(start: f32, end: f32, amount: f32) -> f32 {
    let difference = qvm_float_to_int(f(end - start));
    let absolute = if difference < 0 {
        difference.wrapping_neg()
    } else {
        difference
    };
    f(f(absolute as f32) / amount)
}

/// Checked `memmove` over an edit buffer (`Item_TextField_HandleKey` move).
fn edit_move(buffer: &mut [u8; EDIT_BUFFER_LEN], destination: i32, source: i32, count: i32) -> Result<(), ClientError> {
    if destination < 0
        || source < 0
        || count < 0
        || destination as i64 + count as i64 > buffer.len() as i64
        || source as i64 + count as i64 > buffer.len() as i64
    {
        return Err(bad_ui("Item_TextField_HandleKey memmove exceeds local char[1024]"));
    }
    buffer.copy_within(source as usize..(source + count) as usize, destination as usize);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::menu::{parse_test_menus, SharedUiMenuMemory, UiMemoryAllocation, UiStringReference};
    use super::*;
    use crate::text::draw2d::{CoordinateSpace, DrawCommand, ImagePicture, TextCommandSink};
    use crate::text::q3_font::{FontProfile, GlyphMetrics, RegisteredFont, RegisteredGlyph};
    use qa_core::identity::IdentityOwner;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Shared test handle.
    type Shared<T> = Rc<RefCell<T>>;
    /// Shared log.
    type Log = Shared<Vec<String>>;

    /// Image picture handle.
    fn pic(id: u32) -> PictureAsset {
        PictureAsset::Image(ImagePicture {
            image: id,
            width: 8,
            height: 8,
        })
    }

    /// Fake cvars with a shared map.
    #[derive(Debug)]
    struct FakeCvars {
        /// Shared values.
        map: Shared<HashMap<String, UiCvarValue>>,
    }

    impl UiCvarRegistry for FakeCvars {
        fn get(&self, name: &str) -> Option<UiCvarValue> {
            self.map.borrow().get(name).cloned()
        }

        fn set(&mut self, name: &str, value: &str, _force: bool) {
            let numeric = value.parse::<f32>().unwrap_or(0.0);
            self.map.borrow_mut().insert(
                name.to_string(),
                UiCvarValue {
                    value: value.to_string(),
                    numeric_value: numeric,
                },
            );
        }

        fn reset(&mut self, name: &str, _force: bool) {
            self.map.borrow_mut().remove(name);
        }
    }

    /// Fake command buffer.
    #[derive(Debug)]
    struct FakeCommands {
        /// Shared log.
        log: Log,
    }

    impl UiCommandBuffer for FakeCommands {
        fn append(&mut self, text: &str, _context: &CommandContext) {
            self.log.borrow_mut().push(format!("exec:{text}"));
        }
    }

    /// Fake resources.
    #[derive(Debug)]
    struct FakeResources {
        /// Shared log.
        log: Log,
        /// Pictures by path.
        pictures: HashMap<String, PictureAsset>,
        /// Sounds by path.
        sounds: HashMap<String, PcmSound>,
        /// Models by path.
        models: HashMap<String, SceneModel>,
        /// Handle sequence.
        next: u32,
    }

    impl FakeResources {
        /// Empty fakes.
        fn new(log: Log) -> Self {
            Self {
                log,
                pictures: HashMap::new(),
                sounds: HashMap::new(),
                models: HashMap::new(),
                next: 0,
            }
        }
    }

    impl UiRuntimeResources for FakeResources {
        fn handle_kind(&self) -> UiHandleKind {
            UiHandleKind::Diagnostic
        }

        fn picture_handle(&mut self, _picture: Option<PictureAsset>) -> Result<i32, ClientError> {
            Err(bad_ui(NUMERIC_HANDLES))
        }

        fn picture_for_handle(&self, _handle: i32) -> Result<Option<PictureAsset>, ClientError> {
            Err(bad_ui(NUMERIC_HANDLES))
        }

        fn model_for_handle(&self, _handle: i32) -> Result<SceneModel, ClientError> {
            Err(bad_ui(NUMERIC_HANDLES))
        }

        fn register_font(&mut self, path: Option<&str>, point_size: i32) {
            self.log.borrow_mut().push(format!("font:{path:?}:{point_size}"));
        }

        fn register_picture(&mut self, path: Option<&str>) -> Option<PictureAsset> {
            self.log.borrow_mut().push(format!("picture:{path:?}"));
            let key = path.unwrap_or("null").to_string();
            if let Some(found) = self.pictures.get(&key) {
                return Some(*found);
            }
            self.next += 1;
            let picture = pic(100 + self.next);
            self.pictures.insert(key, picture);
            Some(picture)
        }

        fn registered_picture(&self, path: Option<&str>) -> Option<PictureAsset> {
            self.pictures.get(path.unwrap_or("null")).copied()
        }

        fn register_sound(&mut self, path: Option<&str>) -> Option<PcmSound> {
            self.log.borrow_mut().push(format!("sound:{path:?}"));
            let key = path.unwrap_or("null").to_string();
            if let Some(found) = self.sounds.get(&key) {
                return Some(found.clone());
            }
            self.next += 1;
            let sound = PcmSound {
                path: path.map(str::to_string),
                handle: self.next,
            };
            self.sounds.insert(key, sound.clone());
            Some(sound)
        }

        fn registered_sound(&self, path: Option<&str>) -> Option<PcmSound> {
            self.sounds.get(path.unwrap_or("null")).cloned()
        }

        fn register_model(&mut self, path: Option<&str>) -> SceneModel {
            self.log.borrow_mut().push(format!("model:{path:?}"));
            let key = path.unwrap_or("null").to_string();
            if let Some(found) = self.models.get(&key) {
                return found.clone();
            }
            self.next += 1;
            let model = SceneModel {
                path: path.map(str::to_string),
                handle: self.next,
            };
            self.models.insert(key, model.clone());
            model
        }

        fn registered_model(&self, path: Option<&str>) -> Option<SceneModel> {
            self.models.get(path.unwrap_or("null")).cloned()
        }

        fn prepare_cinematic(&mut self, path: &str) -> UiCinematicAsset {
            self.log.borrow_mut().push(format!("cinematic:{path}"));
            UiCinematicAsset { path: path.to_string() }
        }
    }

    /// Fake audio.
    #[derive(Debug)]
    struct FakeAudio {
        /// Shared log.
        log: Log,
    }

    impl UiRuntimeAudio for FakeAudio {
        fn play_local(&mut self, sound: Option<UiLocalSound>) {
            self.log.borrow_mut().push(format!("play:{sound:?}"));
        }

        fn start_background(&mut self, path: Option<&str>) {
            self.log.borrow_mut().push(format!("bg:{path:?}"));
        }

        fn stop_background(&mut self) {
            self.log.borrow_mut().push("stop-bg".to_string());
        }
    }

    /// Fake cinematics.
    #[derive(Debug)]
    struct FakeCinematics {
        /// Shared log.
        log: Log,
    }

    impl UiRuntimeCinematics for FakeCinematics {
        fn play(&mut self, asset: &UiCinematicAsset, _rect: &UiRect) -> Option<UiCinematicInstance> {
            self.log.borrow_mut().push(format!("cinematic-play:{}", asset.path));
            Some(UiCinematicInstance {
                asset: asset.clone(),
                handle: 7,
            })
        }

        fn run(&mut self, handle: i32, time: i32) {
            self.log.borrow_mut().push(format!("cinematic-run:{handle}:{time}"));
        }

        fn draw(&mut self, handle: i32, _rect: &UiRect, _draw: &mut Draw2D) {
            self.log.borrow_mut().push(format!("cinematic-draw:{handle}"));
        }

        fn stop(&mut self, handle: i32) {
            self.log.borrow_mut().push(format!("cinematic-stop:{handle}"));
        }
    }

    /// Fake feeder.
    #[derive(Debug)]
    struct FakeFeeder {
        /// Shared log.
        log: Log,
        /// Row count.
        count: i32,
    }

    impl UiRuntimeFeeder for FakeFeeder {
        fn count(&mut self, _feeder: f32) -> i32 {
            self.count
        }

        fn item(&mut self, _feeder: f32, index: i32, _column: i32) -> Option<UiRuntimeFeederItem> {
            Some(UiRuntimeFeederItem {
                text: Some(format!("row{index}")),
                picture: None,
            })
        }

        fn image(&mut self, _feeder: f32, _index: i32) -> Option<PictureAsset> {
            None
        }

        fn select(&mut self, feeder: f32, index: i32) {
            self.log.borrow_mut().push(format!("select:{feeder}:{index}"));
        }
    }

    /// Fake owner-draw.
    #[derive(Debug)]
    struct FakeOwnerDraw {
        /// Shared log.
        log: Log,
    }

    impl UiRuntimeOwnerDraw for FakeOwnerDraw {
        fn visible(&mut self, _flags: i32) -> bool {
            true
        }

        fn width(&mut self, _owner_draw: i32, _scale: f32) -> i32 {
            0
        }

        fn value(&mut self, _owner_draw: i32) -> f32 {
            0.0
        }

        fn handle_key(&mut self, _owner_draw: i32, _flags: i32, special: f32, _key: i32) -> UiOwnerDrawKeyResult {
            UiOwnerDrawKeyResult {
                handled: false,
                special,
            }
        }

        fn paint(&mut self, request: UiOwnerDrawPaintRequest<'_, '_>) {
            self.log
                .borrow_mut()
                .push(format!("owner-paint:{}", request.owner_draw));
        }

        fn close_cinematic(&mut self, owner_draw: i32) {
            self.log.borrow_mut().push(format!("owner-close:{owner_draw}"));
        }
    }

    /// Fake external scripts (drains remaining tokens into the log).
    #[derive(Debug)]
    struct FakeExternal {
        /// Shared log.
        log: Log,
    }

    impl UiExternalScriptHost for FakeExternal {
        fn run(&mut self, cursor: &mut dyn UiScriptCursor, context: &UiExternalScriptContext) {
            self.log
                .borrow_mut()
                .push(format!("external:{:?}:{:?}", context.menu_name, context.item_name));
            while let Some(token) = cursor.next_token() {
                self.log.borrow_mut().push(format!("token:{}", token.text));
            }
        }
    }

    /// Fake bindings.
    #[derive(Debug)]
    struct FakeBindings {
        /// Shared bindings.
        map: Shared<HashMap<i32, String>>,
        /// Overstrike state.
        overstrike: bool,
    }

    impl UiRuntimeBindings for FakeBindings {
        fn key_name(&mut self, key: i32) -> String {
            format!("key{key}")
        }

        fn get_binding(&mut self, key: i32) -> String {
            self.map.borrow().get(&key).cloned().unwrap_or_default()
        }

        fn set_binding(&mut self, key: i32, command: &str) {
            self.map.borrow_mut().insert(key, command.to_string());
        }

        fn get_overstrike(&mut self) -> bool {
            self.overstrike
        }

        fn set_overstrike(&mut self, enabled: bool) {
            self.overstrike = enabled;
        }
    }

    /// Null string memory (simulates NULL allocation).
    #[derive(Debug)]
    struct NullMemory;

    impl UiMenuMemory for NullMemory {
        fn allocate(&self, _size: usize) -> Option<usize> {
            None
        }

        fn borrow(&self, _offset: usize, size: usize) -> UiMemoryAllocation {
            UiMemoryAllocation::zeroed(size)
        }

        fn menu_record(&self, _index: usize) -> UiMemoryAllocation {
            UiMemoryAllocation::zeroed(644)
        }

        fn string_alloc(&self, _text: Option<&str>) -> Result<Option<String>, ClientError> {
            Ok(None)
        }

        fn string_alloc_reference(&self, _text: Option<&str>) -> Result<Option<UiStringReference>, ClientError> {
            Ok(None)
        }
    }

    /// Shared test state.
    struct TestState {
        /// Cvar values.
        cvars: Shared<HashMap<String, UiCvarValue>>,
        /// Host log.
        log: Log,
        /// Bindings.
        bindings: Shared<HashMap<i32, String>>,
    }

    /// Test harness.
    struct Harness {
        /// Runtime under test.
        runtime: UiRuntime,
        /// Shared state.
        state: TestState,
    }

    /// Test glyph.
    fn test_glyph() -> RegisteredGlyph {
        RegisteredGlyph {
            metrics: GlyphMetrics {
                height: 8,
                top: 8,
                bottom: 0,
                pitch: 8,
                x_skip: 8,
                image_width: 8,
                image_height: 8,
                s: 0.0,
                t: 0.0,
                s2: 1.0,
                t2: 1.0,
                shader_name: String::new(),
            },
            picture: None,
        }
    }

    /// Test font with full glyph coverage.
    fn test_font(name: &str) -> RegisteredFont {
        RegisteredFont {
            name: name.to_string(),
            glyph_scale: 1.0,
            glyphs: vec![test_glyph(); 256],
        }
    }

    /// Test font set.
    fn test_fonts() -> FontSet {
        FontSet {
            small: test_font("small"),
            normal: test_font("normal"),
            big: test_font("big"),
            profile: FontProfile::Ui,
            small_threshold: 0.5,
            big_threshold: 2.0,
        }
    }

    /// Test widget pictures.
    fn test_widgets() -> UiWidgetAssets {
        UiWidgetAssets {
            white_shader: pic(1),
            gradient_bar: pic(2),
            scroll_bar: pic(3),
            scroll_bar_arrow_down: pic(4),
            scroll_bar_arrow_up: pic(5),
            scroll_bar_arrow_left: pic(6),
            scroll_bar_arrow_right: pic(7),
            scroll_bar_thumb: pic(8),
            slider_bar: pic(9),
            slider_thumb: pic(10),
        }
    }

    /// Parse fixture menus (`(menu_name, menu_source)` pairs) into definitions.
    ///
    /// Each pair becomes `ui/<name>.menu`, loaded through a generated set file.
    fn parse_fixture(menus: &[(&str, &str)]) -> UiMenuDefinitions {
        let mut files = HashMap::new();
        let mut set = String::new();
        for (name, source) in menus {
            set.push_str(&format!("loadmenu {{ \"ui/{name}.menu\" }}\n"));
            files.insert(format!("ui/{name}.menu"), source.to_string());
        }
        files.insert("ui/menus.txt".to_string(), set);
        parse_test_menus(files, "ui/menus.txt").unwrap()
    }

    /// Parse one fixture menu.
    fn parse_menu(name: &str, source: &str) -> UiMenuDefinition {
        parse_fixture(&[(name, source)]).menus.into_iter().next().unwrap()
    }

    /// Empty visible menu source with a fullscreen rect.
    fn empty_menu(name: &str) -> String {
        format!("menuDef {{\n  name \"{name}\"\n  rect 0 0 640 480\n  visible 1\n}}\n")
    }

    /// Menu source with extra header keywords and item sources appended.
    fn menu_with(name: &str, extra: &str, items: &str) -> String {
        format!("menuDef {{\n  name \"{name}\"\n  rect 0 0 640 480\n  visible 1\n{extra}{items}}}\n")
    }

    /// Item source with the standard test rect at `y` plus extra keywords.
    fn item_at(name: &str, item_type: i32, y: f32, extra: &str) -> String {
        format!(
            "  itemDef {{\n    name \"{name}\"\n    type {item_type}\n    rect 10 {y} 200 24\n    visible 1\n{extra}  }}\n"
        )
    }

    /// Menu source holding the given item sources.
    fn menu_items(name: &str, items: &[String]) -> String {
        menu_with(name, "", &items.concat())
    }

    /// Build options with shared state.
    fn build_options(
        definitions: UiMenuDefinitions,
        seed: &dyn Fn(&mut FakeResources),
    ) -> (UiRuntimeOptions, TestState) {
        let owner = IdentityOwner::create("runtime-test").unwrap();
        let cvars: Shared<HashMap<String, UiCvarValue>> = Rc::new(RefCell::new(HashMap::new()));
        let log: Log = Rc::new(RefCell::new(Vec::new()));
        let bindings: Shared<HashMap<i32, String>> = Rc::new(RefCell::new(HashMap::new()));
        let mut resources = FakeResources::new(log.clone());
        seed(&mut resources);
        let paint_log = log.clone();
        let pause_log = log.clone();
        let options = UiRuntimeOptions {
            definitions,
            source_parser: None,
            print: None,
            cvar_value: None,
            cvars: Box::new(FakeCvars { map: cvars.clone() }),
            commands: Box::new(FakeCommands { log: log.clone() }),
            command_context: CommandContext {
                session: owner.session().clone(),
                origin: UiCommandOrigin::LocalConsole,
            },
            resources: Box::new(resources),
            fonts: test_fonts(),
            widget_assets: test_widgets(),
            zero_picture: pic(0),
            audio: Box::new(FakeAudio { log: log.clone() }),
            cinematics: Box::new(FakeCinematics { log: log.clone() }),
            paint_model: Box::new(move |request| {
                paint_log.borrow_mut().push(format!("model:{}", request.angle));
            }),
            context: UiRuntimeContext::Ui {
                bindings: Box::new(FakeBindings {
                    map: bindings.clone(),
                    overstrike: false,
                }),
                pause: Box::new(move |paused| {
                    pause_log.borrow_mut().push(format!("pause:{paused}"));
                }),
            },
            feeder: Box::new(FakeFeeder {
                log: log.clone(),
                count: 5,
            }),
            owner_draw: Box::new(FakeOwnerDraw { log: log.clone() }),
            external_script: Box::new(FakeExternal { log: log.clone() }),
            get_team_color: Box::new(|| Vec4 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
                w: 1.0,
            }),
        };
        (options, TestState { cvars, log, bindings })
    }

    /// Build a harness.
    fn harness(definitions: UiMenuDefinitions) -> Harness {
        let (options, state) = build_options(definitions, &|_| {});
        let runtime = UiRuntime::create(options).unwrap();
        Harness { runtime, state }
    }

    /// Recording sink over a 640x480 target.
    fn test_sink() -> TextCommandSink {
        let owner = IdentityOwner::create("paint-test").unwrap();
        TextCommandSink::new(
            owner.seat(0),
            Rect {
                x: 0.0,
                y: 0.0,
                width: 640.0,
                height: 480.0,
            },
        )
    }

    /// Whether the sink recorded a stretch of a picture.
    fn stretched(sink: &TextCommandSink, picture: PictureAsset) -> bool {
        sink.commands.iter().any(|command| match command {
            DrawCommand::StretchPic { picture: found, .. } => *found == picture,
            _ => false,
        })
    }

    #[test]
    fn create_registers_resources_and_backgrounds() {
        let definitions = parse_fixture(&[(
            "main",
            &menu_with(
                "main",
                "  font \"fonts/a\"\n  background \"pics/bg\"\n",
                &item_at(
                    "model",
                    7,
                    10.0,
                    "    asset_model \"models/c\"\n    focussound \"sounds/b\"\n",
                ),
            ),
        )]);
        let (options, state) = build_options(definitions, &|_| {});
        let runtime = UiRuntime::create(options).unwrap();
        let log = state.log.borrow();
        assert!(log.iter().any(|entry| entry.starts_with("font:")), "{log:?}");
        assert!(log.iter().any(|entry| entry.starts_with("picture:")), "{log:?}");
        assert!(log.iter().any(|entry| entry.starts_with("sound:")), "{log:?}");
        assert!(log.iter().any(|entry| entry.starts_with("model:")), "{log:?}");
        let snapshot = runtime.snapshot().unwrap();
        assert_eq!(snapshot.menus[0].items.len(), 1);
        assert_eq!(snapshot.menus.len(), 1);
    }

    #[test]
    fn activate_show_close_tracks_stack() {
        let mut fixture = harness(parse_fixture(&[
            ("main", &empty_menu("main")),
            ("other", &empty_menu("other")),
        ]));
        assert!(fixture.runtime.show("main").unwrap());
        assert_eq!(fixture.runtime.menu_count().unwrap(), 2);
        assert!(fixture
            .runtime
            .activate(UiMenuSelector::Name("other".to_string()))
            .unwrap());
        let snapshot = fixture.runtime.snapshot().unwrap();
        assert_eq!(snapshot.focused_menu.as_deref(), Some("other"));
        assert_eq!(snapshot.open_stack, vec![Some("main".to_string())]);
        assert!(fixture.runtime.close("other").unwrap());
        let snapshot = fixture.runtime.snapshot().unwrap();
        // The donor never pops the open stack or restores focus on close.
        assert_eq!(snapshot.focused_menu.as_deref(), None);
        assert_eq!(snapshot.open_stack, vec![Some("main".to_string())]);
        assert!(!fixture.runtime.show("missing").unwrap());
        assert!(!fixture.runtime.close("missing").unwrap());
    }

    #[test]
    fn activate_null_clears_focus() {
        let mut fixture = harness(parse_fixture(&[("main", &empty_menu("main"))]));
        assert!(fixture.runtime.show("main").unwrap());
        assert!(!fixture.runtime.activate(UiMenuSelector::Null).unwrap());
        assert_eq!(fixture.runtime.focused_menu_handle().unwrap(), None);
    }

    #[test]
    fn reset_definitions_scopes() {
        let mut fixture = harness(parse_fixture(&[
            ("main", &empty_menu("main")),
            ("other", &empty_menu("other")),
        ]));
        fixture.runtime.show("main").unwrap();
        fixture
            .runtime
            .activate(UiMenuSelector::Name("other".to_string()))
            .unwrap();
        fixture.runtime.reset_definitions(UiDefinitionReset::Menus).unwrap();
        assert_eq!(fixture.runtime.menu_count().unwrap(), 0);
        assert_eq!(fixture.runtime.snapshot().unwrap().open_stack.len(), 1);
        fixture.runtime.reset_definitions(UiDefinitionReset::Strings).unwrap();
        assert!(fixture.runtime.snapshot().unwrap().open_stack.is_empty());
    }

    #[test]
    fn append_reload_guard_memory() {
        let mut fixture = harness(parse_fixture(&[("main", &empty_menu("main"))]));
        fixture
            .runtime
            .append_menu(
                parse_menu("extra", &empty_menu("extra")),
                &UiMenuMemoryOwnership::Unaccounted,
            )
            .unwrap();
        assert_eq!(fixture.runtime.menu_count().unwrap(), 2);
        let other = UiMenuMemoryOwnership::Qvm32 {
            memory: SharedUiMenuMemory::new(NullMemory),
        };
        assert!(fixture
            .runtime
            .append_menu(parse_menu("bad", &empty_menu("bad")), &other)
            .is_err());
        assert!(fixture.runtime.assert_menu_memory(&other).is_err());
        let mut definitions = UiMenuDefinitions::empty();
        definitions.menus = vec![parse_menu("solo", &empty_menu("solo"))];
        fixture.runtime.reload_definitions(definitions).unwrap();
        assert_eq!(fixture.runtime.menu_count().unwrap(), 1);
        let mut foreign = UiMenuDefinitions::empty();
        foreign.memory = other;
        foreign.menus = vec![parse_menu("x", &empty_menu("x"))];
        assert!(fixture.runtime.reload_definitions(foreign).is_err());
    }

    #[test]
    fn key_navigation_runs_item_action() {
        let first = item_at(
            "first",
            1,
            10.0,
            "    text \"first\"\n    action { setcvar picked first }\n",
        );
        let second = item_at(
            "second",
            1,
            40.0,
            "    text \"second\"\n    action { setcvar picked second }\n",
        );
        let items = format!("{first}{second}");
        let mut fixture = harness(parse_fixture(&[("main", &menu_with("main", "", &items))]));
        fixture.runtime.show("main").unwrap();
        fixture.runtime.set_display_cursor(20.0, 15.0).unwrap();
        fixture.runtime.pointer_move(20.0, 15.0).unwrap();
        let snapshot = fixture.runtime.snapshot().unwrap();
        assert_eq!(snapshot.menus[0].cursor_item, 0);
        assert!(fixture
            .runtime
            .handle_key(
                UiKeyEvent::Key {
                    code: KeyCode::Down as i32,
                    down: true
                },
                20.0,
                15.0
            )
            .unwrap());
        let snapshot = fixture.runtime.snapshot().unwrap();
        assert_eq!(snapshot.menus[0].cursor_item, 1);
        assert!(fixture
            .runtime
            .handle_key(
                UiKeyEvent::Key {
                    code: KeyCode::Enter as i32,
                    down: true
                },
                20.0,
                45.0
            )
            .unwrap());
        assert_eq!(
            fixture
                .state
                .cvars
                .borrow()
                .get("picked")
                .map(|found| found.value.clone()),
            Some("second".to_string())
        );
    }

    #[test]
    fn key_validation_rejects_ranges() {
        let mut fixture = harness(parse_fixture(&[("main", &empty_menu("main"))]));
        fixture.runtime.show("main").unwrap();
        assert!(fixture
            .runtime
            .handle_key(UiKeyEvent::Key { code: -1, down: true }, 0.0, 0.0)
            .is_err());
        assert!(fixture
            .runtime
            .handle_key(
                UiKeyEvent::Key {
                    code: 0x8000,
                    down: true
                },
                0.0,
                0.0
            )
            .is_err());
        assert!(fixture
            .runtime
            .handle_key(UiKeyEvent::Character { code: 256 }, 0.0, 0.0)
            .is_err());
        assert!(fixture
            .runtime
            .handle_key(UiKeyEvent::Key { code: 1, down: true }, f32::NAN, 0.0)
            .is_err());
        assert!(fixture.runtime.pointer_move(f32::INFINITY, 0.0).is_err());
        assert!(fixture.runtime.set_display_cursor(0.0, f32::NAN).is_err());
    }

    #[test]
    fn edit_field_types_backspaces_and_escapes() {
        let field = item_at("name", 4, 10.0, "    cvar \"name\"\n");
        let mut fixture = harness(parse_fixture(&[("main", &menu_items("main", &[field]))]));
        fixture.runtime.show("main").unwrap();
        fixture.runtime.set_display_cursor(20.0, 15.0).unwrap();
        fixture.runtime.pointer_move(20.0, 15.0).unwrap();
        fixture
            .runtime
            .handle_key(
                UiKeyEvent::Key {
                    code: KeyCode::Enter as i32,
                    down: true,
                },
                20.0,
                15.0,
            )
            .unwrap();
        // Consumed chars still report unhandled after fall-through, like the donor.
        assert!(!fixture
            .runtime
            .handle_key(UiKeyEvent::Character { code: 97 }, 20.0, 15.0)
            .unwrap());
        assert_eq!(
            fixture
                .state
                .cvars
                .borrow()
                .get("name")
                .map(|found| found.value.clone()),
            Some("a".to_string())
        );
        fixture
            .runtime
            .handle_key(UiKeyEvent::Character { code: 8 }, 20.0, 15.0)
            .unwrap();
        assert_eq!(
            fixture
                .state
                .cvars
                .borrow()
                .get("name")
                .map(|found| found.value.clone()),
            Some(String::new())
        );
        assert!(fixture
            .runtime
            .handle_key(
                UiKeyEvent::Key {
                    code: KeyCode::Escape as i32,
                    down: true
                },
                20.0,
                15.0
            )
            .unwrap());
        assert!(!fixture
            .runtime
            .handle_key(UiKeyEvent::Character { code: 98 }, 20.0, 15.0)
            .unwrap());
        assert_eq!(
            fixture
                .state
                .cvars
                .borrow()
                .get("name")
                .map(|found| found.value.clone()),
            Some(String::new())
        );
    }

    #[test]
    fn yes_no_click_toggles_cvar() {
        let toggle = item_at("toggle", 11, 10.0, "    cvar \"yn\"\n");
        let mut fixture = harness(parse_fixture(&[("main", &menu_items("main", &[toggle]))]));
        fixture.runtime.show("main").unwrap();
        fixture.state.cvars.borrow_mut().insert(
            "yn".to_string(),
            UiCvarValue {
                value: "0".to_string(),
                numeric_value: 0.0,
            },
        );
        fixture.runtime.set_display_cursor(20.0, 15.0).unwrap();
        fixture.runtime.pointer_move(20.0, 15.0).unwrap();
        fixture
            .runtime
            .handle_key(
                UiKeyEvent::Key {
                    code: KeyCode::Mouse1 as i32,
                    down: true,
                },
                20.0,
                15.0,
            )
            .unwrap();
        assert_eq!(
            fixture.state.cvars.borrow().get("yn").map(|found| found.value.clone()),
            Some("1".to_string())
        );
    }

    #[test]
    fn multi_click_cycles_values() {
        let multi = item_at(
            "choice",
            12,
            10.0,
            "    cvar \"mv\"\n    cvarfloatlist { \"low\" 0 \"mid\" 1 \"high\" 2 }\n",
        );
        let mut fixture = harness(parse_fixture(&[("main", &menu_items("main", &[multi]))]));
        fixture.runtime.show("main").unwrap();
        fixture.state.cvars.borrow_mut().insert(
            "mv".to_string(),
            UiCvarValue {
                value: "0".to_string(),
                numeric_value: 0.0,
            },
        );
        fixture.runtime.set_display_cursor(20.0, 15.0).unwrap();
        fixture.runtime.pointer_move(20.0, 15.0).unwrap();
        let click = UiKeyEvent::Key {
            code: KeyCode::Mouse1 as i32,
            down: true,
        };
        fixture.runtime.handle_key(click, 20.0, 15.0).unwrap();
        assert_eq!(
            fixture.state.cvars.borrow().get("mv").map(|found| found.value.clone()),
            Some("1".to_string())
        );
        fixture.runtime.handle_key(click, 20.0, 15.0).unwrap();
        fixture.runtime.handle_key(click, 20.0, 15.0).unwrap();
        assert_eq!(
            fixture.state.cvars.borrow().get("mv").map(|found| found.value.clone()),
            Some("0".to_string())
        );
    }

    #[test]
    fn list_keys_scroll_and_select() {
        let list = item_at(
            "rows",
            6,
            10.0,
            "    feeder 3\n    elementwidth 180\n    elementheight 12\n",
        );
        let mut fixture = harness(parse_fixture(&[("main", &menu_items("main", &[list]))]));
        fixture.runtime.show("main").unwrap();
        fixture.runtime.set_display_cursor(20.0, 15.0).unwrap();
        fixture.runtime.pointer_move(20.0, 15.0).unwrap();
        fixture
            .runtime
            .handle_key(
                UiKeyEvent::Key {
                    code: KeyCode::Down as i32,
                    down: true,
                },
                20.0,
                15.0,
            )
            .unwrap();
        assert!(fixture.state.log.borrow().iter().any(|entry| entry == "select:3:1"));
        fixture.runtime.scroll_feeder(3.0, true, None).unwrap();
        let snapshot = fixture.runtime.snapshot().unwrap();
        assert!(matches!(
            snapshot.menus[0].items[0].behavior,
            UiRuntimeItemBehaviorSnapshot::ListBox { cursor_position: 2, .. }
        ));
        fixture.runtime.set_feeder_selection(3.0, 0, None).unwrap();
        let snapshot = fixture.runtime.snapshot().unwrap();
        assert!(matches!(
            snapshot.menus[0].items[0].behavior,
            UiRuntimeItemBehaviorSnapshot::ListBox {
                start_position: 0,
                cursor_position: 0,
                ..
            }
        ));
    }

    #[test]
    fn binding_capture_assigns_key() {
        let bind = item_at("attack", 13, 10.0, "    cvar \"+attack\"\n");
        let mut fixture = harness(parse_fixture(&[("main", &menu_items("main", &[bind]))]));
        fixture.runtime.show("main").unwrap();
        fixture.runtime.set_display_cursor(20.0, 15.0).unwrap();
        fixture.runtime.pointer_move(20.0, 15.0).unwrap();
        assert!(!fixture.runtime.binding_pending().unwrap());
        fixture
            .runtime
            .handle_key(
                UiKeyEvent::Key {
                    code: KeyCode::Mouse1 as i32,
                    down: true,
                },
                20.0,
                15.0,
            )
            .unwrap();
        assert!(fixture.runtime.binding_pending().unwrap());
        fixture
            .runtime
            .handle_key(UiKeyEvent::Key { code: 97, down: true }, 20.0, 15.0)
            .unwrap();
        assert!(!fixture.runtime.binding_pending().unwrap());
        assert_eq!(
            fixture.state.bindings.borrow().get(&97).cloned(),
            Some("+attack".to_string())
        );
        assert!(fixture
            .state
            .log
            .borrow()
            .iter()
            .any(|entry| entry == "exec:in_restart\n"));
    }

    #[test]
    fn menu_scripts_show_hide_and_set() {
        let panel = item_at("panel", 1, 40.0, "");
        let mut fixture = harness(parse_fixture(&[("main", &menu_items("main", &[panel]))]));
        fixture.runtime.show("main").unwrap();
        fixture
            .runtime
            .run_menu_script("main", &UiScript::from_text("hide panel"))
            .unwrap();
        let snapshot = fixture.runtime.snapshot().unwrap();
        assert_eq!(snapshot.menus[0].items[0].flags & UiWindowFlag::VISIBLE, 0);
        fixture
            .runtime
            .run_menu_script("main", &UiScript::from_text("show panel"))
            .unwrap();
        let snapshot = fixture.runtime.snapshot().unwrap();
        assert_ne!(snapshot.menus[0].items[0].flags & UiWindowFlag::VISIBLE, 0);
        fixture
            .runtime
            .run_menu_script("main", &UiScript::from_text("setcvar speed fast"))
            .unwrap();
        assert_eq!(
            fixture
                .state
                .cvars
                .borrow()
                .get("speed")
                .map(|found| found.value.clone()),
            Some("fast".to_string())
        );
        fixture
            .runtime
            .run_menu_script("main", &UiScript::from_text("setitemcolor panel forecolor 1 0 0 1"))
            .unwrap();
        let snapshot = fixture.runtime.snapshot().unwrap();
        assert_eq!(
            snapshot.menus[0].items[0].fore_color,
            Vec4 {
                x: 1.0,
                y: 0.0,
                z: 0.0,
                w: 1.0,
            }
        );
        fixture
            .runtime
            .run_menu_script("main", &UiScript::from_text("dance panel"))
            .unwrap();
        assert!(fixture.state.log.borrow().iter().any(|entry| entry == "token:panel"));
    }

    #[test]
    fn conditional_open_selects_menu() {
        let mut fixture = harness(parse_fixture(&[
            ("main", &empty_menu("main")),
            ("first", &empty_menu("first")),
            ("second", &empty_menu("second")),
        ]));
        fixture.runtime.show("main").unwrap();
        fixture.state.cvars.borrow_mut().insert(
            "mode".to_string(),
            UiCvarValue {
                value: "1".to_string(),
                numeric_value: 1.0,
            },
        );
        fixture
            .runtime
            .run_menu_script("main", &UiScript::from_text("conditionalopen mode first second"))
            .unwrap();
        assert_eq!(
            fixture.runtime.snapshot().unwrap().focused_menu.as_deref(),
            Some("first")
        );
        fixture.state.cvars.borrow_mut().insert(
            "mode".to_string(),
            UiCvarValue {
                value: "0".to_string(),
                numeric_value: 0.0,
            },
        );
        fixture
            .runtime
            .run_menu_script("main", &UiScript::from_text("conditionalopen mode first second"))
            .unwrap();
        assert_eq!(
            fixture.runtime.snapshot().unwrap().focused_menu.as_deref(),
            Some("second")
        );
    }

    #[test]
    fn frame_paints_to_sink() {
        let label = item_at("label", 0, 10.0, "    text \"label\"\n");
        let rows = "  itemDef {\n    name \"rows\"\n    type 6\n    rect 10 60 200 100\n    visible 1\n    feeder 2\n    elementwidth 180\n    elementheight 12\n  }\n"
            .to_string();
        let items = format!("{label}{rows}");
        let mut fixture = harness(parse_fixture(&[("main", &menu_with("main", "  style 2\n", &items))]));
        fixture.runtime.show("main").unwrap();
        let mut sink = test_sink();
        let frame = UiRuntimeFrame {
            time: 100,
            frame_time: 16,
            draw: Draw2D::new(&mut sink, CoordinateSpace::Stretch640),
        };
        fixture.runtime.frame(frame, 60.0).unwrap();
        assert!(stretched(&sink, pic(2)));
        assert!(stretched(&sink, pic(3)));
        assert!(!sink.commands.is_empty());
    }

    #[test]
    fn snapshot_reports_enable_and_behavior() {
        let gated = item_at(
            "gated",
            4,
            10.0,
            "    cvar \"gated\"\n    cvartest \"mode\"\n    enablecvar { 1 }\n",
        );
        let list = item_at(
            "rows",
            6,
            40.0,
            "    feeder 3\n    elementwidth 180\n    elementheight 12\n",
        );
        let definitions = parse_fixture(&[("main", &menu_items("main", &[gated, list]))]);
        let parsed = definitions.menus[0].items().unwrap();
        assert_eq!(parsed[0].cvar_rule().unwrap().kind(), "enable");
        assert_eq!(parsed[0].cvar_test().as_deref(), Some("mode"));
        let mut fixture = harness(definitions);
        fixture.runtime.show("main").unwrap();
        fixture.state.cvars.borrow_mut().insert(
            "mode".to_string(),
            UiCvarValue {
                value: "1".to_string(),
                numeric_value: 1.0,
            },
        );
        let snapshot = fixture.runtime.snapshot().unwrap();
        assert!(snapshot.menus[0].items[0].enabled);
        assert!(snapshot.menus[0].items[0].shown);
        assert!(matches!(
            snapshot.menus[0].items[0].behavior,
            UiRuntimeItemBehaviorSnapshot::Edit { paint_offset: 0 }
        ));
        assert!(matches!(
            snapshot.menus[0].items[1].behavior,
            UiRuntimeItemBehaviorSnapshot::ListBox { .. }
        ));
        fixture.state.cvars.borrow_mut().insert(
            "mode".to_string(),
            UiCvarValue {
                value: "0".to_string(),
                numeric_value: 0.0,
            },
        );
        let snapshot = fixture.runtime.snapshot().unwrap();
        assert!(!snapshot.menus[0].items[0].enabled);
    }

    #[test]
    fn out_of_bounds_click_closes() {
        let definitions = parse_fixture(&[("popup", &empty_menu("popup"))]);
        let menu = &definitions.menus[0];
        menu.window().set_rect(&UiRect {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        });
        menu.window()
            .set_flags(UiWindowFlag::VISIBLE | UiWindowFlag::HAS_FOCUS | UiWindowFlag::OUT_OF_BOUNDS_CLICK);
        let mut fixture = harness(definitions);
        fixture.runtime.set_display_cursor(500.0, 500.0).unwrap();
        fixture
            .runtime
            .handle_key(
                UiKeyEvent::Key {
                    code: KeyCode::Mouse1 as i32,
                    down: true,
                },
                500.0,
                500.0,
            )
            .unwrap();
        let snapshot = fixture.runtime.snapshot().unwrap();
        assert_eq!(snapshot.menus[0].flags & UiWindowFlag::VISIBLE, 0);
    }

    #[test]
    fn slider_click_sets_cvar() {
        let slider = item_at("volume", 10, 10.0, "    cvarfloat \"vol\" 0 0 10\n");
        let mut fixture = harness(parse_fixture(&[("main", &menu_items("main", &[slider]))]));
        fixture.runtime.show("main").unwrap();
        fixture.runtime.set_display_cursor(58.0, 15.0).unwrap();
        fixture.runtime.pointer_move(58.0, 15.0).unwrap();
        fixture
            .runtime
            .handle_key(
                UiKeyEvent::Key {
                    code: KeyCode::Mouse1 as i32,
                    down: true,
                },
                58.0,
                15.0,
            )
            .unwrap();
        assert_eq!(
            fixture.state.cvars.borrow().get("vol").map(|found| found.value.clone()),
            Some("5.000000".to_string())
        );
    }

    #[test]
    fn model_paint_reports_angle() {
        let model = item_at(
            "player",
            7,
            10.0,
            "    asset_model \"models/x\"\n    model_angle 0\n    model_rotation 10\n",
        );
        let definitions = parse_fixture(&[("main", &menu_items("main", &[model]))]);
        let (options, state) = build_options(definitions, &|resources| {
            resources.models.insert(
                "models/x".to_string(),
                SceneModel {
                    path: Some("models/x".to_string()),
                    handle: 9,
                },
            );
        });
        let mut runtime = UiRuntime::create(options).unwrap();
        runtime.show("main").unwrap();
        let mut sink = test_sink();
        let frame = UiRuntimeFrame {
            time: 100,
            frame_time: 16,
            draw: Draw2D::new(&mut sink, CoordinateSpace::Stretch640),
        };
        runtime.frame(frame, 60.0).unwrap();
        assert!(state.log.borrow().iter().any(|entry| entry == "model:1"));
    }

    #[test]
    fn cinematic_window_plays() {
        let source = menu_with("movie", "  style 5\n  cinematic \"vid.roq\"\n", "");
        let mut fixture = harness(parse_fixture(&[("movie", &source)]));
        fixture.runtime.show("movie").unwrap();
        let mut sink = test_sink();
        let frame = UiRuntimeFrame {
            time: 100,
            frame_time: 16,
            draw: Draw2D::new(&mut sink, CoordinateSpace::Stretch640),
        };
        fixture.runtime.frame(frame, 60.0).unwrap();
        let log = fixture.state.log.borrow();
        assert!(log.iter().any(|entry| entry == "cinematic:vid.roq"), "{log:?}");
        assert!(log.iter().any(|entry| entry == "cinematic-play:vid.roq"), "{log:?}");
        assert!(log.iter().any(|entry| entry == "cinematic-run:7:100"), "{log:?}");
        assert!(log.iter().any(|entry| entry == "cinematic-draw:7"), "{log:?}");
    }

    #[test]
    fn cursor_type_detects_sizer() {
        let fixture = harness(parse_fixture(&[("main", &empty_menu("main"))]));
        assert_eq!(fixture.runtime.cursor_type(0.0, 0.0).unwrap(), UiCursorType::Sizer);
        assert_eq!(fixture.runtime.cursor_type(320.0, 240.0).unwrap(), UiCursorType::Arrow);
    }

    #[test]
    fn dispose_blocks_operations() {
        let mut fixture = harness(parse_fixture(&[("main", &empty_menu("main"))]));
        fixture.runtime.show("main").unwrap();
        fixture.runtime.dispose();
        assert!(fixture.runtime.snapshot().is_err());
        assert!(fixture.runtime.menu_count().is_err());
        assert!(fixture
            .runtime
            .handle_key(UiKeyEvent::Key { code: 1, down: true }, 0.0, 0.0)
            .is_err());
        fixture.runtime.dispose();
    }

    #[test]
    fn script_cursor_tokenizes() {
        let memory = UiMenuMemoryOwnership::Unaccounted;
        let mut cursor =
            RuntimeScriptCursor::new("show panel \"quoted name\" ; extra // trailing", 3, &memory).unwrap();
        assert_eq!(cursor.position(), 0);
        assert_eq!(cursor.remaining(), 5);
        assert_eq!(cursor.peek().map(|token| token.text), Some("show".to_string()));
        assert_eq!(cursor.position(), 0);
        assert_eq!(cursor.string(), Some(Some("show".to_string())));
        assert_eq!(cursor.string(), Some(Some("panel".to_string())));
        assert_eq!(cursor.string(), Some(Some("quoted name".to_string())));
        assert_eq!(cursor.string(), Some(Some(";".to_string())));
        assert_eq!(cursor.string(), Some(Some("extra".to_string())));
        assert_eq!(cursor.string(), None);
        assert_eq!(cursor.position(), 5);
    }

    #[test]
    fn script_cursor_stops_at_line_break() {
        // COM_Parse with line breaks disallowed ends the token stream, like the donor.
        let memory = UiMenuMemoryOwnership::Unaccounted;
        let mut cursor = RuntimeScriptCursor::new("show panel // hide me\n\"quoted name\"", 0, &memory).unwrap();
        assert_eq!(cursor.remaining(), 2);
        assert_eq!(cursor.string(), Some(Some("show".to_string())));
        assert_eq!(cursor.string(), Some(Some("panel".to_string())));
        assert_eq!(cursor.string(), None);
    }

    #[test]
    fn script_cursor_reports_null_allocation() {
        let memory = UiMenuMemoryOwnership::Qvm32 {
            memory: SharedUiMenuMemory::new(NullMemory),
        };
        let mut cursor = RuntimeScriptCursor::new("show panel", 0, &memory).unwrap();
        assert_eq!(cursor.string(), Some(None));
    }

    #[test]
    fn game_numbers_match_bg_lib() {
        assert_eq!(game_atof("1.5").unwrap(), 1.5);
        assert_eq!(game_atof("  -2.25 trailing").unwrap(), -2.25);
        assert_eq!(game_atoi(" -12x").unwrap(), -12);
        assert_eq!(game_atoi("99").unwrap(), 99);
        assert_eq!(game_format("%i", &[GameFormatArg::Int(42)]).unwrap(), "42");
        assert_eq!(game_format("%f", &[GameFormatArg::Float(1.5)]).unwrap(), "1.500000");
        assert_eq!(
            game_format("%s ; ", &[GameFormatArg::Text(Some("exec".to_string()))]).unwrap(),
            "exec ; "
        );
        assert_eq!(
            game_format("fps: %f", &[GameFormatArg::Float(60.0)]).unwrap(),
            "fps: 60.000000"
        );
    }
}
