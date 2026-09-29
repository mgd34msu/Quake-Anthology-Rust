//! Q2 rerelease HUD layout and inventory grammar.
//!
//! Ported from the TypeScript donor's `src/ui/hud/q2-rerelease-layout.ts`
//! (rerelease `cg_screen.cpp`). Executes server-sent layout strings into
//! [`NativeQ2HudOperation`] draw operations and renders the 19-row inventory
//! panel. Frame, operation, arsenal, and inventory-readout types are shared
//! with (and owned by) the `q2_native` sibling; this module owns the
//! rerelease [`NativeQ2HudEnvironment`] services and scoreboard table.

use std::rc::Rc;

use qa_core::math::Vec4;

use super::q2_native::{
    native_q2_hud_stat, NativeInventoryReadout, NativeQ2HudArsenal, NativeQ2HudFrame, NativeQ2HudOperation,
};
use super::token::HudTokenizer;
use crate::error::ClientError;
use crate::hud::game_atoi;

/// Stat holding the packed boss health-bar fractions.
const STAT_HEALTH_BARS: i32 = 52;
/// Stat holding the active weapon index.
const STAT_ACTIVE_WEAPON: i32 = 53;
/// Configstring holding the health-bar title.
const CONFIG_HEALTH_BAR_NAME: i32 = 12104;
/// Configstring holding the story text.
const CONFIG_STORY: i32 = 12105;
/// Configstring base for wheel-weapon warning thresholds.
const CS_WHEEL_WEAPONS: i32 = 12350;
/// Table cell width cap in bytes.
const CELL_BYTES: usize = 23;

/// Text measurer: width and height in HUD units.
pub type Q2HudMeasureFn = Rc<dyn Fn(&str) -> (f32, f32)>;
/// Localizer with positional arguments.
pub type Q2HudLocalizeFn = Rc<dyn Fn(&str, &[String]) -> String>;

/// Scoreboard table accumulated by `start_table`/`table_row`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NativeQ2HudTable {
    /// Rows of truncated cells; row zero holds the headers.
    pub rows: Vec<Vec<String>>,
    /// Per-column pixel widths.
    pub columns: Vec<i32>,
}

/// Font and localization services owned by the caller.
pub struct NativeQ2HudEnvironment {
    /// Scoreboard table state; `None` discards table writes.
    pub table: Option<NativeQ2HudTable>,
    /// Whether scaled-font text renders instead of console text.
    pub use_font: bool,
    /// Scaled-font line height in HUD units.
    pub font_line_height: f32,
    /// Measure text to `(width, height)` in HUD units.
    pub measure: Q2HudMeasureFn,
    /// Localize text with positional arguments.
    pub localize: Q2HudLocalizeFn,
}

/// Truncate a table cell to [`CELL_BYTES`] bytes on a UTF-8 boundary.
fn cell(value: &str) -> String {
    if value.len() <= CELL_BYTES {
        return value.to_string();
    }
    let mut end = CELL_BYTES;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

/// Look up the display name of client `slot` (`skin` part dropped).
fn client_name(frame: &NativeQ2HudFrame, slot: i32) -> Result<String, ClientError> {
    if !(0..256).contains(&slot) {
        return Err(ClientError::BadUi("Q2 HUD client is outside clientinfo".to_string()));
    }
    let config = frame.protocol.layout();
    let info = frame
        .configstrings
        .get(&(config.player_skins + slot))
        .map(String::as_str)
        .unwrap_or("");
    Ok(info.split('\\').next().unwrap_or("").to_string())
}

/// Localize `text`, expanding `##P<slot>` markers to client names.
fn localized(
    frame: &NativeQ2HudFrame,
    localize: &Q2HudLocalizeFn,
    text: &str,
    args: &[String],
) -> Result<String, ClientError> {
    let value = localize(text, args);
    let bytes = value.as_bytes();
    let mut out = String::with_capacity(value.len());
    let mut offset = 0;
    while offset < bytes.len() {
        let rest = &value[offset..];
        let Some(marker) = rest.find("##P") else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..marker]);
        let mut digits = marker + 3;
        while digits < rest.len() && rest.as_bytes()[digits].is_ascii_digit() {
            digits += 1;
        }
        if digits == marker + 3 {
            out.push_str("##P");
            offset += marker + 3;
            continue;
        }
        let slot: i32 = rest[marker + 3..digits]
            .parse()
            .map_err(|_| ClientError::BadUi("Q2 HUD client is outside clientinfo".to_string()))?;
        out.push_str(&client_name(frame, slot)?);
        offset += digits;
    }
    Ok(out)
}

/// Execute a rerelease layout string into draw operations.
pub fn q2_rerelease_layout(
    source: &str,
    frame: &NativeQ2HudFrame,
    width: i32,
    height: i32,
    env: &mut NativeQ2HudEnvironment,
    arsenal: Option<&NativeQ2HudArsenal>,
) -> Result<Vec<NativeQ2HudOperation>, ClientError> {
    let mut out = Vec::new();
    let mut parser = HudTokenizer::new(source, "Q2 rerelease HUD layout");
    let config = frame.protocol.layout();
    let mut conditions: Vec<bool> = Vec::new();
    let localize = Rc::clone(&env.localize);
    let measure_text = Rc::clone(&env.measure);
    let use_font = env.use_font;
    let font_line_height = env.font_line_height;
    macro_rules! next_token {
        () => {
            parser.next(true)?.map(|token| token.value).unwrap_or_default()
        };
    }
    let font_offset = (font_line_height - 8.0) / 2.0;
    let measure = |value: &str, force_font: bool| -> f32 {
        if use_font || force_font {
            measure_text(value).0
        } else {
            value.len() as f32 * 8.0
        }
    };
    let mut x = 0.0f32;
    let mut y = 0.0f32;

    macro_rules! push_text {
        ($value:expr, $alternate:expr, $at_x:expr, $at_y:expr, $force_font:expr) => {{
            let font = use_font || $force_font;
            if font {
                out.push(NativeQ2HudOperation::FontText {
                    x: $at_x,
                    y: $at_y - font_offset,
                    text: $value,
                    alternate: $alternate,
                });
            } else {
                out.push(NativeQ2HudOperation::Text {
                    x: $at_x,
                    y: $at_y,
                    text: $value,
                    alternate: $alternate,
                    shadow: true,
                    xor: true,
                });
            }
        }};
    }

    macro_rules! push_centered {
        ($value:expr, $alternate:expr, $at_x:expr, $at_y:expr) => {{
            let mut line_y = $at_y;
            for line in $value.split('\n') {
                let line = line.to_string();
                let offset = (320.0 - measure(&line, false)) / 2.0;
                push_text!(line, $alternate, $at_x + offset, line_y, false);
                line_y += if use_font { 10.0 } else { 8.0 };
            }
        }};
    }

    macro_rules! push_picture {
        ($name:expr, $at_x:expr, $at_y:expr) => {{
            let name: String = $name;
            if !name.is_empty() {
                out.push(NativeQ2HudOperation::Picture {
                    x: $at_x,
                    y: $at_y,
                    name,
                    anchor_before: false,
                });
            }
        }};
    }

    macro_rules! push_field {
        ($value:expr, $digits:expr, $alternate:expr) => {{
            let count = 5.min($digits);
            if count >= 1 {
                let value_text = ($value as f64).trunc().to_string();
                let length = value_text.len().min(count as usize);
                let mut at_x = x + 2.0 + 16.0 * (count as f32 - length as f32);
                for digit in value_text.chars().take(length) {
                    let name = format!(
                        "{}_{}",
                        if $alternate { "anum" } else { "num" },
                        if digit == '-' {
                            "minus".to_string()
                        } else {
                            digit.to_string()
                        }
                    );
                    push_picture!(name, at_x, y);
                    at_x += 16.0;
                }
            }
        }};
    }

    let stat = |index: i32| -> Result<i32, ClientError> { native_q2_hud_stat(frame, index, arsenal) };
    let config_stat = |index: i32| -> Result<String, ClientError> {
        let value = stat(index)?;
        if value < 0 || value >= config.max_config_strings {
            return Err(ClientError::BadUi(
                "Q2 HUD stat string is outside configstrings".to_string(),
            ));
        }
        Ok(frame.configstrings.get(&value).cloned().unwrap_or_default())
    };
    let count_args = |value: i32, maximum: i32| -> Result<i32, ClientError> {
        if value < 0 || value > maximum {
            return Err(ClientError::BadUi(
                "Q2 HUD argument count exceeds source limits".to_string(),
            ));
        }
        Ok(value)
    };
    let flash = frame.time_ms % 1000 < 500;

    while let Some(token) = parser.next(true)? {
        let command = token.value.as_str();
        let draw = conditions.last().copied().unwrap_or(true);
        match command {
            "if" => {
                let index = game_atoi(&next_token!());
                conditions.push(draw && stat(index)? != 0);
            }
            "ifgef" => {
                let value = game_atoi(&next_token!());
                conditions.push(draw && frame.server_frame >= value);
            }
            "endif" => {
                if conditions.pop().is_none() {
                    return Err(ClientError::BadUi("Q2 HUD endif without matching if".to_string()));
                }
            }
            "xl" | "xr" | "xv" => {
                let value = game_atoi(&next_token!());
                if draw {
                    x = (value
                        + if command == "xr" {
                            width
                        } else if command == "xv" {
                            width / 2 - 160
                        } else {
                            0
                        }) as f32;
                }
            }
            "yt" | "yb" | "yv" => {
                let value = game_atoi(&next_token!());
                if draw {
                    y = (value
                        + if command == "yb" {
                            height
                        } else if command == "yv" {
                            height / 2 - 120
                        } else {
                            0
                        }) as f32;
                }
            }
            "pic" => {
                let index = game_atoi(&next_token!());
                if !draw {
                    continue;
                }
                if index == 2 && arsenal.is_some_and(|bound| bound.ammunition.is_some()) {
                    let ammunition = arsenal.and_then(|bound| bound.ammunition.as_ref());
                    if ammunition.is_some_and(|ammo| ammo.count.is_some())
                        && ammunition.is_some_and(|ammo| ammo.icon.is_some())
                    {
                        let icon = ammunition.and_then(|ammo| ammo.icon.clone());
                        if let Some(icon) = icon {
                            out.push(NativeQ2HudOperation::ArsenalPicture {
                                x,
                                y,
                                resource: icon.resource,
                                aspect: icon.aspect,
                            });
                        }
                    }
                    continue;
                }
                if index == 6 && arsenal.is_some_and(|bound| bound.selected_item.is_some()) {
                    let selected = arsenal.and_then(|bound| bound.selected_item.as_ref());
                    if stat(index)? != 0 {
                        if let Some(icon) = selected.and_then(|item| item.icon.clone()) {
                            out.push(NativeQ2HudOperation::ArsenalPicture {
                                x,
                                y,
                                resource: icon.resource,
                                aspect: icon.aspect,
                            });
                        }
                    }
                    continue;
                }
                let value = stat(index)?;
                if value < 0 || value >= config.max_images {
                    return Err(ClientError::BadUi("Q2 HUD image is outside configstrings".to_string()));
                }
                let name = frame
                    .configstrings
                    .get(&(config.images + value))
                    .cloned()
                    .unwrap_or_default();
                push_picture!(name, x, y);
            }
            "picn" => {
                let value = next_token!();
                if draw {
                    push_picture!(value, x, y);
                }
            }
            "num" => {
                let digits = game_atoi(&next_token!());
                let index = game_atoi(&next_token!());
                if draw {
                    push_field!(stat(index)?, digits, false);
                }
            }
            "lives_num" => {
                let index = game_atoi(&next_token!());
                if draw {
                    let value = stat(index)?;
                    push_field!(0.max(value - 2), 1, value <= 2 && flash);
                }
            }
            "hnum" | "anum" | "rnum" => {
                if !draw {
                    continue;
                }
                let index = if command == "hnum" {
                    1
                } else if command == "anum" {
                    3
                } else {
                    5
                };
                let value = stat(index)?;
                if index != 1 && value < 0 {
                    continue;
                }
                let weapon = stat(STAT_ACTIVE_WEAPON)?;
                let wheel = frame
                    .configstrings
                    .get(&(CS_WHEEL_WEAPONS + weapon))
                    .map(String::as_str)
                    .unwrap_or("");
                let parsed = game_atoi(wheel.split('|').nth(6).unwrap_or("0"));
                let warning = if parsed == 0 { 5 } else { parsed };
                let alternate = if index == 1 {
                    value <= 0 || (value <= 25 && flash)
                } else {
                    index == 3 && value <= warning && flash
                };
                if stat(15)?
                    & (if index == 1 {
                        1
                    } else if index == 3 {
                        4
                    } else {
                        2
                    })
                    != 0
                {
                    push_picture!("field_3".to_string(), x, y);
                }
                push_field!(value, 3, alternate);
            }
            "stat_string" | "loc_stat_string" | "loc_stat_rstring" | "loc_stat_cstring" | "loc_stat_cstring2" => {
                let index = game_atoi(&next_token!());
                if !draw {
                    continue;
                }
                let selected = if index == 51 && stat(index)? != 0 {
                    arsenal.and_then(|bound| bound.selected_item.as_ref())
                } else {
                    None
                };
                let raw = selected.map_or_else(|| config_stat(index), |item| Ok(item.label.clone()))?;
                let value = if command == "stat_string" {
                    raw
                } else {
                    selected.map_or_else(
                        || localized(frame, &localize, &raw, &[]),
                        |item| Ok(item.localized_label.clone()),
                    )?
                };
                if command.starts_with("loc_stat_cstring") {
                    push_centered!(value, command.ends_with('2'), x, y);
                } else {
                    let at_x = x - if command == "loc_stat_rstring" {
                        measure(&value, false)
                    } else {
                        0.0
                    };
                    push_text!(value, false, at_x, y, false);
                }
            }
            "string" | "string2" | "cstring" | "cstring2" => {
                let value = next_token!();
                if draw {
                    if command.starts_with("cstring") {
                        push_centered!(value, command.ends_with('2'), x, y);
                    } else {
                        push_text!(value, command.ends_with('2'), x, y, false);
                    }
                }
            }
            "loc_string" | "loc_string2" | "loc_rstring" | "loc_rstring2" | "loc_cstring" | "loc_cstring2" => {
                let size = count_args(game_atoi(&next_token!()), 7)?;
                let base = next_token!();
                let mut args = Vec::with_capacity(size as usize);
                for _ in 0..size {
                    args.push(next_token!());
                }
                if !draw {
                    continue;
                }
                let value = localized(frame, &localize, &base, &args)?;
                let alternate = command.ends_with('2');
                if command.starts_with("loc_cstring") {
                    push_centered!(value, alternate, x, y);
                } else {
                    let at_x = x - if command.starts_with("loc_rstring") {
                        measure(&value, false)
                    } else {
                        0.0
                    };
                    push_text!(value, alternate, at_x, y, false);
                }
            }
            "client" => {
                let px = game_atoi(&next_token!());
                let py = game_atoi(&next_token!());
                let slot = game_atoi(&next_token!());
                let score = game_atoi(&next_token!());
                let ping = game_atoi(&next_token!());
                if !draw {
                    continue;
                }
                x = (width / 2 - 160 + px + 8) as f32;
                y = (height / 2 - 120 + py + 7) as f32;
                push_text!(client_name(frame, slot)?, false, x + 32.0, y, false);
                push_text!(score.to_string(), !use_font, x + 32.0, y + 10.0, false);
                out.push(NativeQ2HudOperation::SizedPicture {
                    x: x + 96.0,
                    y: y + 10.0,
                    width: 9.0,
                    height: 9.0,
                    name: "ping".to_string(),
                });
                push_text!(
                    ping.to_string(),
                    false,
                    x + if use_font { 107.0 } else { 105.0 },
                    y + 10.0,
                    false
                );
            }
            "ctf" => {
                let px = game_atoi(&next_token!());
                let py = game_atoi(&next_token!());
                let slot = game_atoi(&next_token!());
                let score = game_atoi(&next_token!());
                let ping = 999.min(game_atoi(&next_token!()));
                let icon = next_token!();
                if !draw {
                    continue;
                }
                x = (width / 2 - 160 + px) as f32;
                y = (height / 2 - 120 + py) as f32;
                push_text!(score.to_string(), slot == frame.player_number, x, y, true);
                x += 27.0;
                push_text!(ping.to_string(), slot == frame.player_number, x, y, true);
                x += 27.0;
                push_text!(client_name(frame, slot)?, slot == frame.player_number, x, y, true);
                if !icon.is_empty() {
                    out.push(NativeQ2HudOperation::Picture {
                        x,
                        y,
                        name: icon,
                        anchor_before: true,
                    });
                }
            }
            "time_limit" => {
                let end = game_atoi(&next_token!());
                if !draw || end < frame.server_frame {
                    continue;
                }
                let seconds = (i64::from(end - frame.server_frame) * frame.frame_time_ms.unwrap_or(25) / 1000) as i32;
                let value = localized(
                    frame,
                    &localize,
                    "$g_score_time",
                    &[format!("{:02}:{:02}", seconds / 60, seconds % 60)],
                )?;
                let at_x = x - measure(&value, false);
                push_text!(value, true, at_x, y, false);
            }
            "dogtag" => {
                let slot = game_atoi(&next_token!());
                if !draw {
                    continue;
                }
                client_name(frame, slot)?;
                let info = frame
                    .configstrings
                    .get(&(config.player_skins + slot))
                    .map(String::as_str)
                    .unwrap_or("");
                let tag = info.split('\\').nth(2).filter(|part| !part.is_empty());
                out.push(NativeQ2HudOperation::SizedPicture {
                    x,
                    y,
                    width: 198.0,
                    height: 32.0,
                    name: format!("/tags/{}.pcx", tag.unwrap_or("default")),
                });
            }
            "start_table" => {
                let size = count_args(game_atoi(&next_token!()), 5)?;
                let mut headers = Vec::with_capacity(size as usize);
                for _ in 0..size {
                    headers.push(next_token!());
                }
                if !draw {
                    continue;
                }
                let mut row = Vec::with_capacity(headers.len());
                for header in &headers {
                    row.push(cell(&localized(frame, &localize, header, &[])?));
                }
                let widths: Vec<i32> = row.iter().map(|value| measure(value, true).trunc() as i32).collect();
                if let Some(table) = env.table.as_mut() {
                    table.rows = vec![row];
                    table.columns = widths;
                }
            }
            "table_row" => {
                let size = count_args(game_atoi(&next_token!()), 6)?;
                let mut values = Vec::with_capacity(size as usize);
                for _ in 0..size {
                    values.push(next_token!());
                }
                if !draw {
                    continue;
                }
                let column_count = env.table.as_ref().map_or(0, |table| table.columns.len());
                let row_count = env.table.as_ref().map_or(0, |table| table.rows.len());
                if row_count >= 11 {
                    return Err(ClientError::BadUi("Q2 HUD table exceeds source dimensions".to_string()));
                }
                let width = (size as usize).max(column_count);
                let row: Vec<String> = (0..width)
                    .map(|index| cell(values.get(index).map(String::as_str).unwrap_or("")))
                    .collect();
                let widths: Vec<i32> = row
                    .iter()
                    .take(column_count)
                    .map(|value| measure(value, true).trunc() as i32)
                    .collect();
                if let Some(table) = env.table.as_mut() {
                    table.rows.push(row);
                    for (index, width) in widths.iter().enumerate() {
                        table.columns[index] = table.columns[index].max(*width);
                    }
                }
            }
            "draw_table" => {
                if !draw {
                    continue;
                }
                let (rows, columns) = env.table.as_ref().map_or_else(
                    || (Vec::new(), Vec::new()),
                    |table| (table.rows.clone(), table.columns.clone()),
                );
                let space = measure_text(" ").0.trunc() as i32;
                let table_width = columns.iter().sum::<i32>() + 0.max(columns.len() as i32 - 1) * space;
                let table_height = rows.len() as f32 * (8.0 + font_offset);
                let left = x - (table_width / 2) as f32;
                let top = y + 8.0;
                let mut glyph = |code: u32, px: f32, py: f32| {
                    out.push(NativeQ2HudOperation::Text {
                        x: px,
                        y: py,
                        text: char::from_u32(code).unwrap_or('?').to_string(),
                        alternate: false,
                        shadow: false,
                        xor: false,
                    });
                };
                glyph(18, left - 8.0, top - 8.0);
                glyph(20, left + table_width as f32, top - 8.0);
                glyph(24, left - 8.0, top + table_height);
                glyph(26, left + table_width as f32, top + table_height);
                let mut edge = left;
                while edge < left + table_width as f32 {
                    glyph(19, edge, top - 8.0);
                    glyph(25, edge, top + table_height);
                    edge += 8.0;
                }
                let mut side = top;
                while side < top + table_height {
                    glyph(21, left - 8.0, side);
                    glyph(23, left + table_width as f32, side);
                    side += 8.0;
                }
                out.push(NativeQ2HudOperation::Fill {
                    x: left,
                    y: top,
                    width: table_width as f32,
                    height: table_height,
                    color: Vec4 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                        w: 1.0,
                    },
                });
                let mut px = left;
                for (index, column) in columns.iter().enumerate() {
                    for (row_index, row) in rows.iter().enumerate() {
                        let value = row.get(index).cloned().unwrap_or_default();
                        let offset = if row_index == 0 {
                            (*column as f32 - measure(&value, true)) / 2.0
                        } else if index == 0 {
                            0.0
                        } else {
                            *column as f32 - measure(&value, true)
                        };
                        push_text!(
                            value,
                            row_index == 0,
                            px + offset,
                            top + row_index as f32 * (8.0 + font_offset),
                            true
                        );
                    }
                    px += *column as f32 + space as f32;
                }
            }
            "stat_pname" => {
                let index = game_atoi(&next_token!());
                if draw {
                    push_text!(client_name(frame, stat(index)? - 1)?, false, x, y, false);
                }
            }
            "health_bars" => {
                if !draw {
                    continue;
                }
                let title = localized(
                    frame,
                    &localize,
                    frame
                        .configstrings
                        .get(&CONFIG_HEALTH_BAR_NAME)
                        .map(String::as_str)
                        .unwrap_or(""),
                    &[],
                )?;
                push_centered!(title, false, (width / 2 - 160) as f32, y);
                y += font_line_height;
                let value = stat(STAT_HEALTH_BARS)?;
                let bar_width = width as f32 * 0.5;
                let left = width as f32 * 0.5 - bar_width * 0.5;
                for bar in 0..2 {
                    let packed = ((value as u32) >> (bar * 8)) & 255;
                    if packed & 128 == 0 {
                        continue;
                    }
                    let fraction = (packed & 127) as f32 / 127.0;
                    out.push(NativeQ2HudOperation::Fill {
                        x: left,
                        y,
                        width: bar_width + 1.0,
                        height: 5.0,
                        color: Vec4 {
                            x: 0.0,
                            y: 0.0,
                            z: 0.0,
                            w: 1.0,
                        },
                    });
                    if fraction > 0.0 {
                        out.push(NativeQ2HudOperation::Fill {
                            x: left,
                            y,
                            width: bar_width * fraction,
                            height: 4.0,
                            color: Vec4 {
                                x: 1.0,
                                y: 0.0,
                                z: 0.0,
                                w: 1.0,
                            },
                        });
                    }
                    if fraction < 1.0 {
                        out.push(NativeQ2HudOperation::Fill {
                            x: left + bar_width * fraction,
                            y,
                            width: bar_width * (1.0 - fraction),
                            height: 4.0,
                            color: Vec4 {
                                x: 80.0 / 255.0,
                                y: 80.0 / 255.0,
                                z: 80.0 / 255.0,
                                w: 1.0,
                            },
                        });
                    }
                    y += 12.0;
                }
            }
            "story" => {
                let raw = frame.configstrings.get(&CONFIG_STORY).map(String::as_str).unwrap_or("");
                if raw.is_empty() {
                    continue;
                }
                let value = localized(frame, &localize, raw, &[])?;
                let size = measure_text(&value);
                for (index, line) in value.split('\n').enumerate() {
                    out.push(NativeQ2HudOperation::FontText {
                        x: (width as f32 - measure_text(line).0) / 2.0,
                        y: (height as f32 - size.1) / 2.0 + index as f32 * font_line_height,
                        text: line.to_string(),
                        alternate: false,
                    });
                }
            }
            _ => {}
        }
    }
    if !conditions.is_empty() {
        return Err(ClientError::BadUi("Q2 HUD if without matching endif".to_string()));
    }
    Ok(out)
}

/// Render the 19-row rerelease inventory panel.
pub fn q2_rerelease_inventory(
    frame: &NativeQ2HudFrame,
    width: i32,
    height: i32,
    env: &mut NativeQ2HudEnvironment,
    inventory: Option<&NativeInventoryReadout>,
) -> Result<Vec<NativeQ2HudOperation>, ClientError> {
    let mut out = Vec::new();
    let config = frame.protocol.layout();
    let selected = frame.stats.get(12).copied().unwrap_or(0);
    let localize = Rc::clone(&env.localize);
    let measure_text = Rc::clone(&env.measure);
    let use_font = env.use_font;
    let font_line_height = env.font_line_height;
    let items: Vec<(String, i32, bool)> = inventory.map_or_else(
        || {
            frame
                .inventory
                .iter()
                .enumerate()
                .filter(|(_, count)| **count != 0)
                .map(|(index, count)| {
                    let name = frame
                        .configstrings
                        .get(&(config.items + index as i32))
                        .cloned()
                        .unwrap_or_default();
                    (name, *count, index as i32 == selected)
                })
                .collect()
        },
        |readout| {
            readout
                .items
                .iter()
                .map(|row| (row.label.clone(), row.count, row.item == readout.selected))
                .collect()
        },
    );
    let selected_row = if inventory.is_none() {
        if selected >= 0 && (selected as usize) < frame.inventory.len() {
            frame.inventory[..selected as usize]
                .iter()
                .filter(|count| **count != 0)
                .count()
        } else {
            0
        }
    } else {
        items.iter().position(|(_, _, selected)| *selected).unwrap_or(0)
    };
    let top = 0.max(items.len() as i32 - 19).min(selected_row as i32 - 9).max(0);
    let x = (width / 2 - 128) as f32;
    let y = (height / 2 - 108) as f32;
    out.push(NativeQ2HudOperation::Picture {
        x,
        y: y + 8.0,
        name: "inventory".to_string(),
        anchor_before: false,
    });
    for (row, (name, count, selected)) in items.iter().skip(top as usize).take(19).enumerate() {
        let name = localized(frame, &localize, name, &[])?;
        let py = y + 27.0 + row as f32 * 8.0;
        if *selected && (frame.time_ms.wrapping_mul(10) as i32 & 1) != 0 {
            out.push(NativeQ2HudOperation::Text {
                x: x + 14.0,
                y: py,
                text: "\u{000f}".to_string(),
                alternate: false,
                shadow: false,
                xor: false,
            });
        }
        if use_font {
            let count_text = count.to_string();
            let offset = (font_line_height - 8.0) / 2.0;
            out.push(NativeQ2HudOperation::FontText {
                x: x + 222.0 - measure_text(&count_text).0,
                y: py - offset,
                text: count_text,
                alternate: *selected,
            });
            out.push(NativeQ2HudOperation::FontText {
                x: x + 38.0,
                y: py - offset,
                text: name,
                alternate: *selected,
            });
        } else {
            out.push(NativeQ2HudOperation::Text {
                x: x + 22.0,
                y: py,
                text: format!("{count:>3} {name}"),
                alternate: *selected,
                shadow: false,
                xor: true,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::super::q2_native::{AmmunitionView, ArsenalIcon, NativeInventoryItem};
    use super::*;
    use crate::ui::types::{Q2ProtocolFamily, ResourceId};

    fn test_frame() -> NativeQ2HudFrame {
        NativeQ2HudFrame {
            protocol: Q2ProtocolFamily::Rerelease,
            stats: vec![0; 64],
            configstrings: BTreeMap::new(),
            layout: String::new(),
            inventory: vec![0; 256],
            player_number: 0,
            server_frame: 100,
            time_ms: 100,
            frame_time_ms: Some(25),
        }
    }

    fn test_env() -> NativeQ2HudEnvironment {
        NativeQ2HudEnvironment {
            table: Some(NativeQ2HudTable::default()),
            use_font: false,
            font_line_height: 10.0,
            measure: Rc::new(|text: &str| (text.len() as f32 * 8.0, 10.0)),
            localize: Rc::new(|text: &str, args: &[String]| {
                if text == "$g_score_time" {
                    return format!("Time: {}", args.first().map(String::as_str).unwrap_or(""));
                }
                let mut value = text.to_string();
                for (index, arg) in args.iter().enumerate() {
                    value = value.replace(&format!("{{{index}}}"), arg);
                }
                value
            }),
        }
    }

    fn run(source: &str, frame: &NativeQ2HudFrame, env: &mut NativeQ2HudEnvironment) -> Vec<NativeQ2HudOperation> {
        q2_rerelease_layout(source, frame, 640, 480, env, None).expect("layout")
    }

    #[test]
    fn positions_and_strings() {
        let frame = test_frame();
        let mut env = test_env();
        let ops = run("xl 10 yb -20 string hello string2 alt", &frame, &mut env);
        assert_eq!(
            ops,
            vec![
                NativeQ2HudOperation::Text {
                    x: 10.0,
                    y: 460.0,
                    text: "hello".to_string(),
                    alternate: false,
                    shadow: true,
                    xor: true,
                },
                NativeQ2HudOperation::Text {
                    x: 10.0,
                    y: 460.0,
                    text: "alt".to_string(),
                    alternate: true,
                    shadow: true,
                    xor: true,
                },
            ]
        );
    }

    #[test]
    fn centered_text_accounts_for_measure() {
        let frame = test_frame();
        let mut env = test_env();
        let ops = run("xl 0 yt 0 cstring ab", &frame, &mut env);
        assert_eq!(
            ops,
            vec![NativeQ2HudOperation::Text {
                x: (320.0 - 16.0) / 2.0,
                y: 0.0,
                text: "ab".to_string(),
                alternate: false,
                shadow: true,
                xor: true,
            }]
        );
    }

    #[test]
    fn conditionals_gate_draws() {
        let mut frame = test_frame();
        frame.stats[7] = 1;
        let mut env = test_env();
        let ops = run(
            "if 7 string shown endif if 8 string hidden endif ifgef 50 string gated endif",
            &frame,
            &mut env,
        );
        let texts: Vec<&str> = ops
            .iter()
            .map(|op| match op {
                NativeQ2HudOperation::Text { text, .. } | NativeQ2HudOperation::FontText { text, .. } => text.as_str(),
                _ => "",
            })
            .collect();
        assert_eq!(texts, vec!["shown", "gated"]);
    }

    #[test]
    fn endif_without_if_fails() {
        let frame = test_frame();
        let mut env = test_env();
        let error = q2_rerelease_layout("endif", &frame, 640, 480, &mut env, None).expect_err("must fail");
        assert_eq!(error.to_string(), "Q2 HUD endif without matching if");
        let error = q2_rerelease_layout("if 1 string x", &frame, 640, 480, &mut env, None).expect_err("must fail");
        assert_eq!(error.to_string(), "Q2 HUD if without matching endif");
    }

    #[test]
    fn stat_outside_playerstate_fails() {
        let frame = test_frame();
        let mut env = test_env();
        let error =
            q2_rerelease_layout("if 99 string x endif", &frame, 640, 480, &mut env, None).expect_err("must fail");
        assert_eq!(error.to_string(), "Q2 HUD stat 99 is outside playerstate");
    }

    #[test]
    fn pic_resolves_configstring_image() {
        let mut frame = test_frame();
        frame.stats[4] = 3;
        frame.configstrings.insert(10302 + 3, "p_pic".to_string());
        let mut env = test_env();
        let ops = run("pic 4 picn direct", &frame, &mut env);
        assert_eq!(
            ops,
            vec![
                NativeQ2HudOperation::Picture {
                    x: 0.0,
                    y: 0.0,
                    name: "p_pic".to_string(),
                    anchor_before: false,
                },
                NativeQ2HudOperation::Picture {
                    x: 0.0,
                    y: 0.0,
                    name: "direct".to_string(),
                    anchor_before: false,
                },
            ]
        );
    }

    #[test]
    fn pic_outside_images_fails() {
        let mut frame = test_frame();
        frame.stats[4] = 512;
        let mut env = test_env();
        let error = q2_rerelease_layout("pic 4", &frame, 640, 480, &mut env, None).expect_err("must fail");
        assert_eq!(error.to_string(), "Q2 HUD image is outside configstrings");
    }

    #[test]
    fn num_draws_digit_pictures() {
        let mut frame = test_frame();
        frame.stats[9] = -42;
        let mut env = test_env();
        let ops = run("num 3 9", &frame, &mut env);
        assert_eq!(
            ops,
            vec![
                NativeQ2HudOperation::Picture {
                    x: 2.0,
                    y: 0.0,
                    name: "num_minus".to_string(),
                    anchor_before: false,
                },
                NativeQ2HudOperation::Picture {
                    x: 18.0,
                    y: 0.0,
                    name: "num_4".to_string(),
                    anchor_before: false,
                },
                NativeQ2HudOperation::Picture {
                    x: 34.0,
                    y: 0.0,
                    name: "num_2".to_string(),
                    anchor_before: false,
                },
            ]
        );
    }

    #[test]
    fn hnum_flashes_low_health_with_field() {
        let mut frame = test_frame();
        frame.stats[1] = 20;
        frame.stats[15] = 1;
        frame.time_ms = 100;
        let mut env = test_env();
        let ops = run("hnum", &frame, &mut env);
        assert!(ops.iter().any(|op| matches!(
            op,
            NativeQ2HudOperation::Picture { name, .. } if name == "field_3"
        )));
        assert!(ops.iter().any(|op| matches!(
            op,
            NativeQ2HudOperation::Picture { name, .. } if name == "anum_2"
        )));
    }

    #[test]
    fn stat_string_and_localized_variants() {
        let mut frame = test_frame();
        frame.stats[10] = 200;
        frame.configstrings.insert(200, "raw ##P1".to_string());
        frame.configstrings.insert(11582 + 1, "bob\\male".to_string());
        let mut env = test_env();
        let ops = run("stat_string 10 loc_stat_string 10", &frame, &mut env);
        assert_eq!(
            ops,
            vec![
                NativeQ2HudOperation::Text {
                    x: 0.0,
                    y: 0.0,
                    text: "raw ##P1".to_string(),
                    alternate: false,
                    shadow: true,
                    xor: true,
                },
                NativeQ2HudOperation::Text {
                    x: 0.0,
                    y: 0.0,
                    text: "raw bob".to_string(),
                    alternate: false,
                    shadow: true,
                    xor: true,
                },
            ]
        );
    }

    #[test]
    fn stat_string_outside_configstrings_fails() {
        let mut frame = test_frame();
        frame.stats[10] = 12448;
        let mut env = test_env();
        let error = q2_rerelease_layout("stat_string 10", &frame, 640, 480, &mut env, None).expect_err("must fail");
        assert_eq!(error.to_string(), "Q2 HUD stat string is outside configstrings");
    }

    #[test]
    fn client_row_layout() {
        let mut frame = test_frame();
        frame.configstrings.insert(11582 + 2, "carol\\female".to_string());
        let mut env = test_env();
        let ops = run("client 0 0 2 15 42", &frame, &mut env);
        assert_eq!(ops.len(), 4);
        assert!(matches!(
            &ops[0],
            NativeQ2HudOperation::Text { text, x, y, .. }
                if text == "carol" && *x == 200.0 && *y == 127.0
        ));
        assert!(matches!(
            &ops[2],
            NativeQ2HudOperation::SizedPicture { name, width, height, .. }
                if name == "ping" && *width == 9.0 && *height == 9.0
        ));
    }

    #[test]
    fn client_outside_clientinfo_fails() {
        let frame = test_frame();
        let mut env = test_env();
        let error = q2_rerelease_layout("client 0 0 300 0 0", &frame, 640, 480, &mut env, None).expect_err("must fail");
        assert_eq!(error.to_string(), "Q2 HUD client is outside clientinfo");
    }

    #[test]
    fn ctf_row_anchors_icon_before() {
        let mut frame = test_frame();
        frame.player_number = 1;
        frame.configstrings.insert(11582 + 1, "dave\\male".to_string());
        let mut env = test_env();
        let ops = run("ctf 0 0 1 7 50 flag", &frame, &mut env);
        assert_eq!(ops.len(), 4);
        assert!(matches!(
            &ops[3],
            NativeQ2HudOperation::Picture { name, anchor_before, .. }
                if name == "flag" && *anchor_before
        ));
        assert!(ops
            .iter()
            .take(3)
            .all(|op| matches!(op, NativeQ2HudOperation::FontText { alternate: true, .. })));
    }

    #[test]
    fn time_limit_formats_clock() {
        let frame = test_frame();
        let mut env = test_env();
        let ops = run("time_limit 7000", &frame, &mut env);
        assert_eq!(ops.len(), 1);
        assert!(matches!(
            &ops[0],
            NativeQ2HudOperation::Text { text, alternate: true, .. } if text == "Time: 02:52"
        ));
    }

    #[test]
    fn dogtag_defaults_missing_tag() {
        let mut frame = test_frame();
        frame.configstrings.insert(11582, "erin".to_string());
        let mut env = test_env();
        let ops = run("dogtag 0", &frame, &mut env);
        assert_eq!(
            ops,
            vec![NativeQ2HudOperation::SizedPicture {
                x: 0.0,
                y: 0.0,
                width: 198.0,
                height: 32.0,
                name: "/tags/default.pcx".to_string(),
            }]
        );
    }

    #[test]
    fn table_draws_box_and_cells() {
        let frame = test_frame();
        let mut env = test_env();
        let ops = run(
            "start_table 2 Name Score table_row 2 alice 10 draw_table",
            &frame,
            &mut env,
        );
        assert!(ops.iter().any(|op| matches!(
            op,
            NativeQ2HudOperation::Fill { width, height, .. } if *width == 88.0 && *height == 18.0
        )));
        let glyphs: Vec<&str> = ops
            .iter()
            .filter_map(|op| match op {
                NativeQ2HudOperation::Text {
                    text,
                    shadow: false,
                    xor: false,
                    ..
                } if text.len() == 1 => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(glyphs.contains(&"\u{12}"));
        assert!(glyphs.contains(&"\u{1a}"));
        let cells: Vec<&str> = ops
            .iter()
            .filter_map(|op| match op {
                NativeQ2HudOperation::FontText { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(cells, vec!["Name", "alice", "Score", "10"]);
        let table = env.table.as_ref().expect("table");
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.columns, vec![40, 40]);
    }

    #[test]
    fn table_cell_truncates_on_utf8_boundary() {
        assert_eq!(cell("short"), "short");
        assert_eq!(cell("abcdefghijklmnopqrstuvwxyz"), "abcdefghijklmnopqrstuvw");
        assert_eq!(cell("ééééééééééééé"), "ééééééééééé");
    }

    #[test]
    fn table_row_overflow_fails() {
        let frame = test_frame();
        let mut env = test_env();
        env.table = Some(NativeQ2HudTable {
            rows: vec![vec!["h".to_string()]; 11],
            columns: vec![8],
        });
        let error = q2_rerelease_layout("table_row 1 v", &frame, 640, 480, &mut env, None).expect_err("must fail");
        assert_eq!(error.to_string(), "Q2 HUD table exceeds source dimensions");
    }

    #[test]
    fn argument_count_overflow_fails() {
        let frame = test_frame();
        let mut env = test_env();
        let error = q2_rerelease_layout("start_table 6 a", &frame, 640, 480, &mut env, None).expect_err("must fail");
        assert_eq!(error.to_string(), "Q2 HUD argument count exceeds source limits");
    }

    #[test]
    fn health_bars_draw_title_and_fills() {
        let mut frame = test_frame();
        frame.configstrings.insert(CONFIG_HEALTH_BAR_NAME, "Boss".to_string());
        frame.stats[STAT_HEALTH_BARS as usize] = 0xC0;
        let mut env = test_env();
        let ops = run("health_bars", &frame, &mut env);
        assert!(matches!(
            &ops[0],
            NativeQ2HudOperation::Text { text, .. } if text == "Boss"
        ));
        let fills: Vec<&NativeQ2HudOperation> = ops
            .iter()
            .filter(|op| matches!(op, NativeQ2HudOperation::Fill { .. }))
            .collect();
        assert_eq!(fills.len(), 3);
        assert!(matches!(
            fills[1],
            NativeQ2HudOperation::Fill { color, height: 4.0, .. }
                if *color == Vec4 { x: 1.0, y: 0.0, z: 0.0, w: 1.0 }
        ));
    }

    #[test]
    fn story_centers_lines() {
        let mut frame = test_frame();
        frame.configstrings.insert(CONFIG_STORY, "one\ntwo".to_string());
        let mut env = test_env();
        let ops = run("story", &frame, &mut env);
        assert_eq!(ops.len(), 2);
        assert!(matches!(
            &ops[0],
            NativeQ2HudOperation::FontText { text, x, .. }
                if text == "one" && *x == (640.0 - 24.0) / 2.0
        ));
        let empty = test_frame();
        let mut env = test_env();
        assert!(run("story", &empty, &mut env).is_empty());
    }

    #[test]
    fn arsenal_stat_overrides() {
        let frame = test_frame();
        let arsenal = NativeQ2HudArsenal {
            ammunition: Some(AmmunitionView {
                count: Some(7),
                icon: None,
            }),
            ..NativeQ2HudArsenal::default()
        };
        assert_eq!(native_q2_hud_stat(&frame, 2, Some(&arsenal)).expect("stat"), 1);
        assert_eq!(native_q2_hud_stat(&frame, 3, Some(&arsenal)).expect("stat"), 7);
        let hidden = NativeQ2HudArsenal {
            ammunition: Some(AmmunitionView {
                count: None,
                icon: None,
            }),
            ..NativeQ2HudArsenal::default()
        };
        assert_eq!(native_q2_hud_stat(&frame, 2, Some(&hidden)).expect("stat"), 0);
        assert_eq!(native_q2_hud_stat(&frame, 3, Some(&hidden)).expect("stat"), -1);
    }

    #[test]
    fn pic_uses_arsenal_picture() {
        let frame = test_frame();
        let mut env = test_env();
        let resource = ResourceId::new("resource:ammo").expect("resource");
        let arsenal = NativeQ2HudArsenal {
            ammunition: Some(AmmunitionView {
                count: Some(5),
                icon: Some(ArsenalIcon {
                    resource: resource.clone(),
                    aspect: 2.0,
                }),
            }),
            ..NativeQ2HudArsenal::default()
        };
        let ops = q2_rerelease_layout("pic 2", &frame, 640, 480, &mut env, Some(&arsenal)).expect("layout");
        assert_eq!(
            ops,
            vec![NativeQ2HudOperation::ArsenalPicture {
                x: 0.0,
                y: 0.0,
                resource,
                aspect: 2.0,
            }]
        );
    }

    #[test]
    fn inventory_lists_counts_without_font() {
        let mut frame = test_frame();
        frame.stats[12] = 1;
        frame.inventory[0] = 3;
        frame.inventory[1] = 12;
        frame.configstrings.insert(11326, "Shells".to_string());
        frame.configstrings.insert(11326 + 1, "Rockets".to_string());
        frame.time_ms = 0;
        let mut env = test_env();
        let ops = q2_rerelease_inventory(&frame, 640, 480, &mut env, None).expect("inventory");
        assert_eq!(ops.len(), 3);
        assert!(matches!(
            &ops[1],
            NativeQ2HudOperation::Text { text, alternate: false, xor: true, .. }
                if text == "  3 Shells"
        ));
        assert!(matches!(
            &ops[2],
            NativeQ2HudOperation::Text { text, alternate: true, .. } if text == " 12 Rockets"
        ));
    }

    #[test]
    fn inventory_uses_readout_selection() {
        let frame = test_frame();
        let mut env = test_env();
        env.use_font = true;
        let readout = NativeInventoryReadout {
            items: vec![
                NativeInventoryItem {
                    label: "Cells".to_string(),
                    count: 50,
                    item: 0,
                },
                NativeInventoryItem {
                    label: "Slugs".to_string(),
                    count: 9,
                    item: 1,
                },
            ],
            selected: 1,
        };
        let ops = q2_rerelease_inventory(&frame, 640, 480, &mut env, Some(&readout)).expect("inventory");
        assert_eq!(ops.len(), 5);
        assert!(matches!(
            &ops[4],
            NativeQ2HudOperation::FontText { text, alternate: true, .. } if text == "Slugs"
        ));
    }
}
