//! Q1 rerelease weapon wheel (`wwheel.txt`) grammar.
//!
//! Ported from the TypeScript donor's `src/ui/hud/q1-wheel.ts`. Parses the
//! `slot N { ... }` blocks into [`Q1WheelSlot`] values and projects them onto
//! the shared [`WheelItem`](super::wheel::WheelItem) presentation rows.

use std::rc::Rc;

use super::token::HudTokenizer;
use super::wheel::WheelItem;
use crate::ui::types::ResourceId;

/// One parsed `slot N { ... }` block from `wwheel.txt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1WheelSlot {
    /// Slot ordinal from the block header.
    pub slot: i32,
    /// `impulse` console impulse, if present and well-formed.
    pub impulse: Option<i32>,
    /// `icon` image path, if present and well-formed.
    pub icon: Option<String>,
    /// `icon_sel` selected image path, if present and well-formed.
    pub selected_icon: Option<String>,
    /// `ammoicon` image path, if present and well-formed.
    pub ammo_icon: Option<String>,
    /// `entvaroffs` source progs entity byte offset, if present and well-formed.
    pub entity_variable_byte_offset: Option<i32>,
    /// `weaponnum` inventory bit, if present and well-formed.
    pub weapon_bits: Option<i32>,
    /// Unrecognized `key values...` lines in file order.
    pub unknown: Vec<(String, Vec<String>)>,
}

/// Parsed wheel plus the donor's non-fatal error lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseQ1Wheel {
    /// Slots parsed before the first fatal error, if any.
    pub slots: Vec<Q1WheelSlot>,
    /// Donor-format error lines (fatal errors stop the parse).
    pub errors: Vec<String>,
}

/// Reads a source progs entity float by byte offset (`entvaroffs`).
pub type EntityFloatFn = Rc<dyn Fn(i32) -> f32>;
/// Renders a [`Q1WheelSlot`] label.
pub type Q1WheelLabelFn = Rc<dyn Fn(&Q1WheelSlot) -> String>;
/// Resolves an icon path to a resource, if available.
pub type Q1WheelImageFn = Rc<dyn Fn(&str) -> Option<ResourceId>>;

/// Live state needed to project [`Q1WheelSlot`] values onto [`WheelItem`] rows.
#[derive(Clone)]
pub struct Q1WheelState {
    /// Inventory bitmask tested against each slot's `weaponnum`.
    pub item_bits: u32,
    /// Reads a source progs entity float by byte offset (`entvaroffs`).
    pub entity_float: EntityFloatFn,
    /// Renders a slot label.
    pub label: Q1WheelLabelFn,
    /// Resolves an icon path to a resource, if available.
    pub image: Q1WheelImageFn,
}

/// Parse `wwheel.txt` text into slots and donor-format errors.
///
/// Fatal errors (`expected slot N {`, unclosed slots, overlong tokens) stop the
/// parse; per-field errors record a line and leave that field empty.
pub fn parse_q1_weapon_wheel(text: &str) -> ParseQ1Wheel {
    let mut tokenizer = HudTokenizer::new(text, "wwheel.txt");
    let mut slots = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    loop {
        let header = match tokenizer.next(true) {
            Err(error) => {
                errors.push(error.to_string());
                break;
            }
            Ok(None) => break,
            Ok(Some(header)) => header,
        };
        let ordinal = match tokenizer.next(true) {
            Err(error) => {
                errors.push(error.to_string());
                break;
            }
            Ok(ordinal) => ordinal,
        };
        let opening = match tokenizer.next(true) {
            Err(error) => {
                errors.push(error.to_string());
                break;
            }
            Ok(opening) => opening,
        };
        let slot = ordinal.as_ref().and_then(|token| js_integer(&token.value));
        let opening_is_brace = opening.as_ref().is_some_and(|token| token.value == "{");
        let Some(slot) = slot else {
            errors.push(format!("line {}: expected slot N {{", header.line));
            break;
        };
        if header.value != "slot" || ordinal.is_none() || !opening_is_brace {
            errors.push(format!("line {}: expected slot N {{", header.line));
            break;
        }

        let mut fields: Vec<(String, Vec<String>)> = Vec::new();
        let mut closed = false;
        'keys: while !closed {
            let key = match tokenizer.next(true) {
                Err(error) => {
                    errors.push(error.to_string());
                    break 'keys;
                }
                Ok(key) => key,
            };
            let Some(key) = key else {
                errors.push(format!("line {}: unclosed slot {slot}", header.line));
                break;
            };
            if key.value == "}" {
                closed = true;
                break;
            }
            let mut values: Vec<String> = Vec::new();
            loop {
                match tokenizer.next(false) {
                    Err(error) => {
                        errors.push(error.to_string());
                        break 'keys;
                    }
                    Ok(None) => break,
                    Ok(Some(value)) => {
                        if value.value == "}" {
                            closed = true;
                            break;
                        }
                        values.push(value.value);
                    }
                }
            }
            set_field(&mut fields, key.value.clone(), values);
        }

        let impulse = take_integer(&mut fields, slot, &mut errors, "impulse");
        let icon = take_string(&mut fields, slot, &mut errors, "icon");
        let selected_icon = take_string(&mut fields, slot, &mut errors, "icon_sel");
        let ammo_icon = take_string(&mut fields, slot, &mut errors, "ammoicon");
        let entity_variable_byte_offset = take_integer(&mut fields, slot, &mut errors, "entvaroffs");
        let weapon_bits = take_integer(&mut fields, slot, &mut errors, "weaponnum");

        slots.push(Q1WheelSlot {
            slot,
            impulse,
            icon,
            selected_icon,
            ammo_icon,
            entity_variable_byte_offset,
            weapon_bits,
            unknown: fields,
        });
        if !closed {
            break;
        }
    }

    ParseQ1Wheel { slots, errors }
}

/// Project parsed slots onto wheel presentation rows.
///
/// `entvaroffs` remains the source progs entity byte offset, not a host actor
/// field index. A slot is owned when its `weaponnum` bit is set in
/// `item_bits`; it has ammo when it tracks no entity float or that float is
/// above zero.
pub fn q1_wheel_items(slots: &[Q1WheelSlot], state: &Q1WheelState) -> Vec<WheelItem> {
    slots
        .iter()
        .map(|slot| {
            let count = slot
                .entity_variable_byte_offset
                .map(|offset| (state.entity_float)(offset));
            WheelItem {
                id: format!("q1-wheel:{}", slot.slot),
                source_ordinal: slot.slot,
                sort_order: slot.slot,
                label: (state.label)(slot),
                owned: slot.weapon_bits.is_some_and(|bits| state.item_bits & bits as u32 != 0),
                has_ammo: count.is_none_or(|count| count > 0.0),
                count,
                warning_count: 0,
                icon: slot.icon.as_ref().and_then(|path| (state.image)(path)),
                selected_icon: slot.selected_icon.as_ref().and_then(|path| (state.image)(path)),
            }
        })
        .collect()
}

/// Insert or overwrite a raw `key values...` field, keeping first position.
fn set_field(fields: &mut Vec<(String, Vec<String>)>, key: String, values: Vec<String>) {
    if let Some(entry) = fields.iter_mut().find(|entry| entry.0 == key) {
        entry.1 = values;
    } else {
        fields.push((key, values));
    }
}

/// Remove and return a raw field's values, if present.
fn take_field(fields: &mut Vec<(String, Vec<String>)>, key: &str) -> Option<Vec<String>> {
    let index = fields.iter().position(|entry| entry.0 == key)?;
    Some(fields.remove(index).1)
}

/// Take a single-valued string field, recording an arity error when needed.
///
/// The key is consumed even on error so it never lands in `unknown`.
fn take_string(
    fields: &mut Vec<(String, Vec<String>)>,
    slot: i32,
    errors: &mut Vec<String>,
    key: &str,
) -> Option<String> {
    let values = take_field(fields, key)?;
    if values.len() != 1 {
        errors.push(format!("slot {slot}: {key} expects one value"));
        return None;
    }
    values.into_iter().next()
}

/// Take a single-valued integer field, recording arity and type errors.
fn take_integer(
    fields: &mut Vec<(String, Vec<String>)>,
    slot: i32,
    errors: &mut Vec<String>,
    key: &str,
) -> Option<i32> {
    let value = take_string(fields, slot, errors, key)?;
    match js_integer(&value) {
        Some(numeric) => Some(numeric),
        None => {
            errors.push(format!("slot {slot}: {key} expects an integer"));
            None
        }
    }
}

/// Parse a donor `Number(text)` integer into `i32`.
///
/// Follows the donor's `Number(...)` plus `Number.isInteger` check: surrounding
/// whitespace is ignored, an empty value is zero, `0x`/`0b`/`0o` literals use
/// their radix, and anything else must scan as a finite whole number that fits
/// in `i32`.
fn js_integer(text: &str) -> Option<i32> {
    let trimmed = text.trim_matches(|character: char| character == '\u{FEFF}' || character.is_whitespace());
    if trimmed.is_empty() {
        return Some(0);
    }
    for (prefix, radix) in [("0x", 16), ("0X", 16), ("0b", 2), ("0B", 2), ("0o", 8), ("0O", 8)] {
        if let Some(digits) = trimmed.strip_prefix(prefix) {
            if digits.is_empty() {
                return None;
            }
            let value = i64::from_str_radix(digits, radix).ok()?;
            return i32::try_from(value).ok();
        }
    }
    let value: f64 = trimmed.parse().ok()?;
    if !value.is_finite() || value.fract() != 0.0 {
        return None;
    }
    if !(f64::from(i32::MIN) <= value && value <= f64::from(i32::MAX)) {
        return None;
    }
    Some(value as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state(item_bits: u32) -> Q1WheelState {
        Q1WheelState {
            item_bits,
            entity_float: Rc::new(|offset| offset as f32 * 2.0),
            label: Rc::new(|slot| format!("weapon {}", slot.slot)),
            image: Rc::new(|path| ResourceId::new(&format!("resource:{path}")).ok()),
        }
    }

    #[test]
    fn parses_full_slots() {
        let parsed = parse_q1_weapon_wheel(
            "slot 2 {\nimpulse 2\nicon w_shot\nicon_sel w_shot_sel\nammoicon a_shell\nentvaroffs 12\nweaponnum 2\n}\n",
        );
        assert!(parsed.errors.is_empty(), "errors: {:?}", parsed.errors);
        assert_eq!(parsed.slots.len(), 1);
        let slot = &parsed.slots[0];
        assert_eq!(slot.slot, 2);
        assert_eq!(slot.impulse, Some(2));
        assert_eq!(slot.icon.as_deref(), Some("w_shot"));
        assert_eq!(slot.selected_icon.as_deref(), Some("w_shot_sel"));
        assert_eq!(slot.ammo_icon.as_deref(), Some("a_shell"));
        assert_eq!(slot.entity_variable_byte_offset, Some(12));
        assert_eq!(slot.weapon_bits, Some(2));
        assert!(slot.unknown.is_empty());
    }

    #[test]
    fn parses_multiple_slots_with_comments_and_quotes() {
        let parsed = parse_q1_weapon_wheel(
            "// wheel\nslot 1 {\nimpulse 1\nicon \"rocket launcher\"\n}\n/* block */ slot 3 {\nweaponnum 8\n}\n",
        );
        assert!(parsed.errors.is_empty(), "errors: {:?}", parsed.errors);
        assert_eq!(parsed.slots.len(), 2);
        assert_eq!(parsed.slots[0].icon.as_deref(), Some("rocket launcher"));
        assert_eq!(parsed.slots[1].weapon_bits, Some(8));
    }

    #[test]
    fn bad_header_reports_line_and_stops() {
        let parsed = parse_q1_weapon_wheel("slot 1 {\nimpulse 1\n}\nnope 2 {\n}\nslot 3 {\n}\n");
        assert_eq!(parsed.errors, vec!["line 4: expected slot N {".to_string()]);
        assert_eq!(parsed.slots.len(), 1);
    }

    #[test]
    fn non_integer_slot_reports_header_error() {
        let parsed = parse_q1_weapon_wheel("slot two {\n}\n");
        assert_eq!(parsed.errors, vec!["line 1: expected slot N {".to_string()]);
        assert!(parsed.slots.is_empty());
    }

    #[test]
    fn missing_brace_reports_header_error() {
        let parsed = parse_q1_weapon_wheel("slot 1\n");
        assert_eq!(parsed.errors, vec!["line 1: expected slot N {".to_string()]);
        assert!(parsed.slots.is_empty());
    }

    #[test]
    fn unclosed_slot_reports_header_line() {
        let parsed = parse_q1_weapon_wheel("slot 4 {\nimpulse 4\n");
        assert_eq!(parsed.errors, vec!["line 1: unclosed slot 4".to_string()]);
        assert_eq!(parsed.slots.len(), 1);
        assert_eq!(parsed.slots[0].impulse, Some(4));
    }

    #[test]
    fn field_arity_and_integer_errors() {
        let parsed = parse_q1_weapon_wheel("slot 1 {\nimpulse 1 2\nweaponnum abc\nentvaroffs 1.5\n}\n");
        assert_eq!(
            parsed.errors,
            vec![
                "slot 1: impulse expects one value".to_string(),
                "slot 1: entvaroffs expects an integer".to_string(),
                "slot 1: weaponnum expects an integer".to_string(),
            ]
        );
        assert_eq!(parsed.slots[0].impulse, None);
        assert_eq!(parsed.slots[0].weapon_bits, None);
        assert_eq!(parsed.slots[0].entity_variable_byte_offset, None);
    }

    #[test]
    fn unknown_fields_are_preserved_in_order() {
        let parsed = parse_q1_weapon_wheel("slot 1 {\nimpulse 1\ncustom a b\nmystery\n}\n");
        assert!(parsed.errors.is_empty(), "errors: {:?}", parsed.errors);
        assert_eq!(
            parsed.slots[0].unknown,
            vec![
                ("custom".to_string(), vec!["a".to_string(), "b".to_string()]),
                ("mystery".to_string(), Vec::new()),
            ]
        );
    }

    #[test]
    fn inline_close_keeps_partial_values() {
        let parsed = parse_q1_weapon_wheel("slot 1 {\nimpulse 2 }\n");
        assert!(parsed.errors.is_empty(), "errors: {:?}", parsed.errors);
        assert_eq!(parsed.slots[0].impulse, Some(2));
    }

    #[test]
    fn items_project_bits_counts_and_images() {
        let parsed = parse_q1_weapon_wheel(
            "slot 1 {\nimpulse 1\nicon w_axe\nicon_sel w_axe_sel\nentvaroffs 4\nweaponnum 1\n}\nslot 2 {\nweaponnum 2\n}\n",
        );
        assert!(parsed.errors.is_empty(), "errors: {:?}", parsed.errors);
        let items = q1_wheel_items(&parsed.slots, &test_state(1));
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, "q1-wheel:1");
        assert_eq!((items[0].source_ordinal, items[0].sort_order), (1, 1));
        assert_eq!(items[0].label, "weapon 1");
        assert!(items[0].owned);
        assert!(items[0].has_ammo);
        assert_eq!(items[0].count, Some(8.0));
        assert_eq!(items[0].warning_count, 0);
        assert_eq!(items[0].icon.as_ref().map(ResourceId::as_str), Some("resource:w_axe"));
        assert_eq!(
            items[0].selected_icon.as_ref().map(ResourceId::as_str),
            Some("resource:w_axe_sel")
        );
        assert!(!items[1].owned);
        assert!(items[1].has_ammo);
        assert_eq!(items[1].count, None);
        assert_eq!(items[1].icon, None);
    }

    #[test]
    fn items_flag_zero_ammo() {
        let parsed = parse_q1_weapon_wheel("slot 5 {\nentvaroffs 0\nweaponnum 4\n}\n");
        assert!(parsed.errors.is_empty(), "errors: {:?}", parsed.errors);
        let state = Q1WheelState {
            item_bits: 4,
            entity_float: Rc::new(|_| 0.0),
            label: Rc::new(|slot| format!("slot {}", slot.slot)),
            image: Rc::new(|_| None),
        };
        let items = q1_wheel_items(&parsed.slots, &state);
        assert!(items[0].owned);
        assert!(!items[0].has_ammo);
        assert_eq!(items[0].count, Some(0.0));
    }

    #[test]
    fn js_integer_vectors() {
        assert_eq!(js_integer("12"), Some(12));
        assert_eq!(js_integer("-3"), Some(-3));
        assert_eq!(js_integer("  7  "), Some(7));
        assert_eq!(js_integer("\"\""), None);
        assert_eq!(js_integer(""), Some(0));
        assert_eq!(js_integer("0x10"), Some(16));
        assert_eq!(js_integer("1e3"), Some(1000));
        assert_eq!(js_integer("1.5"), None);
        assert_eq!(js_integer("abc"), None);
        assert_eq!(js_integer("12abc"), None);
        assert_eq!(js_integer("Infinity"), None);
        assert_eq!(js_integer("99999999999999999999"), None);
    }
}
