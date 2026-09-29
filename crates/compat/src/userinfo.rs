//! Userinfo helpers: value lookup, printing, and name cleaning.
//!
//! Donor provenance: `src/core/info-string.ts` (`infoValueForKey`,
//! `printInfo`) and `src/content/q3/team-arena/client-admission.ts`
//! (`clientInfoValue`, `cleanClientName`, `validInfo`, `byteBuffer`).
//! Pair mutation (`Info_SetValueForKey`) already lives in
//! `qa_core::cvar::set_info_value`; this module only reads and cleans.

use crate::CompatError;

/// Client userinfo bound (`MAX_INFO_STRING`).
pub const MAX_INFO_STRING: usize = 1024;
/// Engine info-string bound (`infoValueForKey` default).
pub const MAX_ENGINE_INFO_STRING: usize = 8192;
/// Netname buffer size (`MAX_NETNAME`, including the terminator).
pub const MAX_NETNAME: usize = 36;

/// Whether an info string is transmittable (no `"` or `;`).
#[must_use]
pub fn valid_info(info: &str) -> bool {
    !info.contains('"') && !info.contains(';')
}

/// Truncate at NUL, bound to `capacity - 1` chars, require byte chars.
fn byte_buffer(value: &str, capacity: usize, label: &str) -> Result<String, CompatError> {
    let visible = value.split('\0').next().unwrap_or("");
    let bounded: String = visible.chars().take(capacity - 1).collect();
    if !bounded.chars().all(|c| (c as u32) <= 0xFF) {
        return Err(CompatError::NonByteChars(label.to_string()));
    }
    Ok(bounded)
}

/// First ASCII case-insensitive match in a `\key\value` string.
fn scan_info_value(text: &str, key: &str) -> String {
    let fold = |value: &str| {
        value
            .chars()
            .map(|c| {
                if c.is_ascii_uppercase() {
                    (c as u8 + 32) as char
                } else {
                    c
                }
            })
            .collect::<String>()
    };
    let folded_key = fold(key);
    let mut cursor = usize::from(text.starts_with('\\'));
    let bytes = text.as_bytes();
    while cursor < bytes.len() {
        let rest = &text[cursor..];
        let Some(separator) = rest.find('\\') else {
            return String::new();
        };
        let name = &rest[..separator];
        let after = &rest[separator + 1..];
        let end = after.find('\\').unwrap_or(after.len());
        if fold(name) == folded_key {
            return after[..end].to_string();
        }
        cursor += separator + 1 + end + 1;
    }
    String::new()
}

/// Engine `Info_ValueForKey`: NUL-truncated, first ASCII-insensitive key.
///
/// `maximum` must be within `1..=8192`.
pub fn info_value_for_key(input: &str, wanted: &str, maximum: usize) -> Result<String, CompatError> {
    if !(1..=MAX_ENGINE_INFO_STRING).contains(&maximum) {
        return Err(CompatError::BadInfoBound);
    }
    let text = input.split('\0').next().unwrap_or("");
    let key = wanted.split('\0').next().unwrap_or("");
    if text.chars().count() >= maximum {
        return Err(CompatError::OversizeInfo);
    }
    for value in [text, key] {
        if !value.chars().all(|c| (c as u32) <= 0xFF) {
            return Err(CompatError::NonByteChars("Info_ValueForKey".to_string()));
        }
    }
    Ok(scan_info_value(text, key))
}

/// Client-sized lookup for engine-owned raw userinfo bytes (1024 bound).
pub fn client_info_value(info: &str, wanted: &str) -> Result<String, CompatError> {
    let text = byte_buffer(info, MAX_INFO_STRING, "userinfo")?;
    let key = byte_buffer(wanted, MAX_INFO_STRING, "userinfo key")?;
    Ok(scan_info_value(&text, &key))
}

/// `Info_Print`: padded keys, values, and `MISSING VALUE` tails.
pub fn print_info(text: &str, print: &mut dyn FnMut(&str)) -> Result<(), CompatError> {
    let mut cursor = usize::from(text.starts_with('\\'));
    while cursor < text.len() {
        let rest = &text[cursor..];
        let separator = rest.find('\\');
        let key = &rest[..separator.unwrap_or(rest.len())];
        if key.chars().count() >= 512 {
            return Err(CompatError::PrintOverflow("key"));
        }
        print(&format!("{key:20}"));
        let Some(separator) = separator else {
            print("MISSING VALUE\n");
            return Ok(());
        };
        let after = &rest[separator + 1..];
        let end = after.find('\\').unwrap_or(after.len());
        let value = &after[..end];
        if value.chars().count() >= 512 {
            return Err(CompatError::PrintOverflow("value"));
        }
        print(&format!("{value}\n"));
        cursor += separator + 1 + end + 1;
    }
    Ok(())
}

/// The 36-byte netname cleaner (`ClientCleanName`).
///
/// Strips leading spaces, drops black (`^0`-class) color codes and
/// trailing carets, collapses runs past three spaces, caps output at 35
/// characters, and defaults empty/colorless names to `UnnamedPlayer`.
pub fn clean_client_name(input: &str) -> Result<String, CompatError> {
    let source = byte_buffer(input, MAX_INFO_STRING, "client name")?;
    let chars: Vec<char> = source.chars().collect();
    let output_size = MAX_NETNAME - 1;
    let mut output = String::new();
    let mut out_len = 0usize;
    let mut colorless = 0usize;
    let mut spaces = 0usize;
    let mut cursor = 0usize;
    while cursor < chars.len() {
        let character = chars[cursor] as u32;
        cursor += 1;
        if out_len == 0 && character == 32 {
            continue;
        }
        if character == 94 {
            if cursor == chars.len() {
                break;
            }
            let color = chars[cursor] as u32;
            if (color.wrapping_sub(48)) & 7 == 0 {
                cursor += 1;
                continue;
            }
            if out_len > output_size - 2 {
                break;
            }
            output.push('^');
            output.push(chars[cursor]);
            out_len += 2;
            cursor += 1;
            continue;
        }
        if character == 32 {
            spaces += 1;
            if spaces > 3 {
                continue;
            }
        } else {
            spaces = 0;
        }
        if out_len > output_size - 1 {
            break;
        }
        output.push(chars[cursor - 1]);
        out_len += 1;
        colorless += 1;
    }
    if out_len == 0 || colorless == 0 {
        return Ok("UnnamedPlayer".to_string());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_lookup_is_first_match_case_insensitive() {
        let info = "\\name\\bob\\NAME\\alice\\team\\red";
        assert_eq!(info_value_for_key(info, "name", 8192).unwrap(), "bob");
        assert_eq!(info_value_for_key(info, "TEAM", 8192).unwrap(), "red");
        assert_eq!(info_value_for_key(info, "missing", 8192).unwrap(), "");
        assert_eq!(info_value_for_key("name\\bob", "name", 8192).unwrap(), "bob");
        assert_eq!(info_value_for_key("name", "name", 8192).unwrap(), "");
        assert_eq!(info_value_for_key("\\a\\1\0\\a\\2", "a", 8192).unwrap(), "1");
        assert!(info_value_for_key(info, "name", 0).is_err());
        assert!(info_value_for_key(info, "name", 8193).is_err());
        assert!(info_value_for_key(&"\\k\\".to_string().repeat(2000), "k", 8192).is_ok());
        assert!(info_value_for_key(&"x".repeat(8192), "k", 8192).is_err());
        assert!(info_value_for_key("\\k\\é", "k", 8192).is_ok());
        assert!(info_value_for_key("\\k\\€", "k", 8192).is_err());
        assert_eq!(client_info_value(info, "name").unwrap(), "bob");
        assert!(valid_info(info));
        assert!(!valid_info("\\a\\b;c"));
        assert!(!valid_info("\\a\\\"b"));
    }

    #[test]
    fn print_info_pads_and_flags_missing_values() {
        let mut out = String::new();
        print_info("\\name\\bob\\lonely", &mut |text| out.push_str(text)).unwrap();
        assert_eq!(out, "name                bob\nlonely              MISSING VALUE\n");
        let mut over = String::new();
        assert!(print_info(&format!("\\{}\\v", "k".repeat(512)), &mut |text| over.push_str(text)).is_err());
    }

    #[test]
    fn clean_client_name_matches_source_cleaner() {
        assert_eq!(clean_client_name("  ^1Bob").unwrap(), "^1Bob");
        assert_eq!(clean_client_name("^0hi").unwrap(), "hi");
        assert_eq!(clean_client_name("ab^").unwrap(), "ab");
        assert_eq!(clean_client_name("a    b").unwrap(), "a   b");
        assert_eq!(clean_client_name("").unwrap(), "UnnamedPlayer");
        assert_eq!(clean_client_name("   ").unwrap(), "UnnamedPlayer");
        assert_eq!(clean_client_name("^1").unwrap(), "UnnamedPlayer");
        assert_eq!(clean_client_name(&"a".repeat(40)).unwrap(), "a".repeat(35));
        assert_eq!(clean_client_name("a\0b").unwrap(), "a");
    }
}
