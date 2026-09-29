//! Bot info strings from `src/bots/behavior/q3/info.ts`.
//!
//! Q3 `\\key\\value` info strings carry client userinfo. Setting a key
//! replaces the pair in place; overlong results truncate at the 1024
//! client-userinfo limit.

/// Maximum info string length.
pub const MAX_INFO_LENGTH: usize = 1024;

/// Get the value for a key in a `\\key\\value` info string.
#[must_use]
pub fn info_value_for_key(info: &str, key: &str) -> String {
    let parts: Vec<&str> = info.split('\\').filter(|part| !part.is_empty()).collect();
    for pair in parts.chunks(2) {
        if pair.len() == 2 && pair[0] == key {
            return pair[1].to_owned();
        }
    }
    String::new()
}

/// Set a key/value pair in a `\\key\\value` info string.
#[must_use]
pub fn info_set_value_for_key(info: &str, key: &str, value: &str) -> String {
    if key.is_empty() || key.contains('\\') || value.contains('\\') {
        return info.to_owned();
    }
    let mut pairs: Vec<(&str, &str)> = Vec::new();
    let parts: Vec<&str> = info.split('\\').collect();
    for pair in parts.chunks(2) {
        if pair.len() == 2 && !pair[0].is_empty() && pair[0] != key {
            pairs.push((pair[0], pair[1]));
        }
    }
    pairs.push((key, value));
    let mut out = String::new();
    for (pair_key, pair_value) in pairs {
        let segment = format!("\\{pair_key}\\{pair_value}");
        if out.len() + segment.len() > MAX_INFO_LENGTH {
            break;
        }
        out.push_str(&segment);
    }
    out
}
