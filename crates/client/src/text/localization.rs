//! Quake rerelease localization grammar and substitutions.
//!
//! Donor provenance: `src/text/localization.ts` (`Loc_Parse`,
//! `comParseToken`, `Loc_ParseInto`, `LocalizationTable`).
//!
//! Positions are Unicode-scalar indices; the donor uses UTF-16 units,
//! which agree on the basic multilingual plane where localization
//! strings live.

use std::collections::BTreeMap;

use qa_core::identity::SeatId;

/// Reborrow an optional log for forwarding (`as_deref_mut` on a
/// `&mut dyn` payload trips `needless_option_as_deref`, and the
/// reborrowed lifetime cannot be named inline).
fn reborrow_log<'a, 'b, 'c>(log: &'a mut Option<&'b mut (dyn LibLog + 'c)>) -> Option<&'a mut (dyn LibLog + 'c)> {
    log.as_mut().map(|log| &mut **log)
}

/// Maximum key bytes.
pub const MAX_LOC_KEY: usize = 64;
/// Maximum format bytes.
pub const MAX_LOC_FORMAT: usize = 1024;
/// Maximum arguments.
pub const MAX_LOC_ARGS: usize = 8;
/// Maximum string bytes.
pub const MAX_STRING_CHARS: usize = 1024;

/// Known languages (`LOC_KNOWN_LANGUAGES`).
pub const LOC_KNOWN_LANGUAGES: [&str; 6] = ["english", "french", "german", "italian", "russian", "spanish"];

/// A log sink (`LibLog`).
pub trait LibLog {
    /// Warn.
    fn warn(&mut self, text: &str);
    /// Info.
    fn info(&mut self, text: &str);
}

/// Byte-truncate with UTF-8 backoff (`strlcpy`).
fn strlcpy(src: &str, size: i64) -> String {
    if size <= 0 {
        return String::new();
    }
    let bytes = src.as_bytes();
    if bytes.len() < size as usize {
        return src.to_string();
    }
    let mut end = (size.max(0) - 1).max(0) as usize;
    while end > 0 && (bytes[end] & 0xc0) == 0x80 {
        end -= 1;
    }
    String::from_utf8_lossy(&bytes[..end.min(bytes.len())]).into_owned()
}

fn strnlcpy(src: &str, count: usize, size: i64) -> String {
    if size <= 0 {
        return String::new();
    }
    let end: usize = src
        .char_indices()
        .take(count)
        .last()
        .map_or(0, |(index, ch)| index + ch.len_utf8());
    let head = if count >= src.chars().count() { src } else { &src[..end] };
    strlcpy(head, size)
}

fn strlcat(dst: &str, src: &str, size: i64) -> String {
    format!("{dst}{}", strlcpy(src, size - dst.len() as i64))
}

fn strnlcat(dst: &str, src: &str, count: usize, size: i64) -> String {
    format!("{dst}{}", strnlcpy(src, count, size - dst.len() as i64))
}

/// A format argument (`LocArg`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LocArg {
    arg_index: usize,
    start: usize,
    end: usize,
}

/// A stored string (`LocString`).
#[derive(Debug, Clone, PartialEq, Eq)]
struct LocString {
    format: String,
    arguments: Vec<LocArg>,
}

/// Parse a format (`Loc_Parse`).
fn loc_parse(format: &str) -> Result<Vec<LocArg>, String> {
    let chars: Vec<char> = format.chars().collect();
    let mut arg_index_state: i32 = 0;
    let mut rover = 0usize;
    let mut args: Vec<LocArg> = Vec::new();
    loop {
        if rover >= chars.len() {
            break;
        }
        if chars[rover] == '{' {
            let arg_start = rover;
            rover += 1;
            if rover < chars.len() && chars[rover] == '{' {
                continue;
            }
            if args.len() == MAX_LOC_ARGS {
                return Err("too many arguments".to_string());
            }
            let mut arg = LocArg {
                arg_index: 0,
                start: arg_start,
                end: 0,
            };
            let rest: String = chars[rover..].iter().collect();
            let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
            let end_offset = rover + digits.chars().count();
            if end_offset == rover {
                if arg_index_state == -1 {
                    return Err("encountered sequential argument, but has positional args".to_string());
                }
                arg.arg_index = (arg_index_state as usize) & 0xff;
                arg_index_state += 1;
            } else {
                if arg_index_state > 0 {
                    return Err("encountered positional argument, but has sequential args".to_string());
                }
                arg.arg_index = digits.parse::<usize>().unwrap_or(0) & 0xff;
                arg_index_state = -1;
            }
            rover = end_offset.saturating_sub(1);
            loop {
                if rover >= chars.len() {
                    return Err("EOF before end of argument found".to_string());
                }
                rover += 1;
                if rover < chars.len() && chars[rover] != '}' {
                    continue;
                }
                if rover >= chars.len() {
                    return Err("EOF before end of argument found".to_string());
                }
                let arg_end = rover;
                rover += 1;
                if rover < chars.len() && chars[rover] == '}' {
                    continue;
                }
                arg.end = arg_end + 1;
                break;
            }
            args.push(arg);
        } else {
            rover += 1;
        }
    }
    args.sort_by_key(|arg| arg.start);
    Ok(args)
}

/// Whether a string has arguments (`Loc_HasArguments`).
fn loc_has_arguments(base: &str) -> bool {
    let chars: Vec<char> = base.chars().collect();
    let mut index = 0usize;
    while index < chars.len() {
        if chars[index] == '{' {
            index += 1;
            if index >= chars.len() {
                return false;
            }
            if chars[index] != '{' {
                return true;
            }
        }
        index += 1;
    }
    false
}

struct TokenState<'a> {
    bytes: &'a [u8],
    index: usize,
}

fn parse_escape(state: &mut TokenState) -> Option<u8> {
    let code = *state.bytes.get(state.index)?;
    state.index += 1;
    if code == 0 {
        return None;
    }
    match code {
        b'n' => Some(0x0a),
        b't' => Some(0x09),
        b'r' => Some(0x0d),
        _ => Some(code),
    }
}

fn com_parse_token(state: &mut TokenState, size: usize, escape: bool) -> String {
    loop {
        let mut byte = state.bytes.get(state.index).copied().unwrap_or(0);
        while byte <= 32 {
            if byte == 0 {
                return String::new();
            }
            state.index += 1;
            byte = state.bytes.get(state.index).copied().unwrap_or(0);
        }
        if byte == b'/' && state.bytes.get(state.index + 1) == Some(&b'/') {
            state.index += 2;
            while state.bytes.get(state.index).copied().unwrap_or(0) != 0
                && state.bytes.get(state.index).copied().unwrap_or(0) != b'\n'
            {
                state.index += 1;
            }
            continue;
        }
        if byte == b'/' && state.bytes.get(state.index + 1) == Some(&b'*') {
            state.index += 2;
            while state.bytes.get(state.index).copied().unwrap_or(0) != 0 {
                if state.bytes.get(state.index) == Some(&b'*') && state.bytes.get(state.index + 1) == Some(&b'/') {
                    state.index += 2;
                    break;
                }
                state.index += 1;
            }
            continue;
        }
        break;
    }
    let byte = state.bytes.get(state.index).copied().unwrap_or(0);
    if byte == b'=' {
        state.index += 1;
        return "=".to_string();
    }
    if byte == b'"' {
        state.index += 1;
        let mut result = Vec::new();
        loop {
            let byte = state.bytes.get(state.index).copied().unwrap_or(0);
            state.index += 1;
            if byte == b'"' || byte == 0 {
                return strlcpy(&String::from_utf8_lossy(&result), size as i64);
            }
            if byte == b'\\' && escape {
                match parse_escape(state) {
                    Some(escaped) => result.push(escaped),
                    None => return strlcpy(&String::from_utf8_lossy(&result), size as i64),
                }
            } else {
                result.push(byte);
            }
        }
    }
    let mut result = Vec::new();
    loop {
        let mut byte = state.bytes.get(state.index).copied().unwrap_or(0);
        if byte == b'\\' && escape {
            match parse_escape(state) {
                Some(escaped) => byte = escaped,
                None => break,
            }
        }
        result.push(byte);
        state.index += 1;
        byte = state.bytes.get(state.index).copied().unwrap_or(0);
        if byte <= 32 || byte == b'=' || byte == 0 {
            break;
        }
    }
    strlcpy(&String::from_utf8_lossy(&result), size as i64)
}

/// Reload options (`LocReloadOptions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LocReloadOptions {
    /// Platform tag.
    pub platform: Option<&'static str>,
    /// Duplicate-key policy override.
    pub duplicate_keys: Option<DuplicateKeys>,
}

/// Duplicate-key policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuplicateKeys {
    /// First wins.
    First,
    /// Last wins.
    Last,
}

fn strip_platform_tag(token: &str) -> String {
    let mut text = token;
    if text.starts_with('<') {
        text = &text[1..];
    }
    if text.ends_with('>') {
        text = &text[..text.len() - 1];
    }
    text.to_ascii_lowercase()
}

/// Merge one file into a table (`Loc_ParseInto` merge path).
fn loc_merge_into<'l>(
    bytes: &[u8],
    options: &LocReloadOptions,
    mut log: Option<&mut (dyn LibLog + 'l)>,
    table: &mut BTreeMap<String, LocString>,
    last_wins: bool,
) -> usize {
    let platform = options.platform.map(str::to_ascii_lowercase);
    let text = String::from_utf8_lossy(bytes).into_owned();
    let mut state = TokenState {
        bytes: text.as_bytes(),
        index: 0,
    };
    let mut seen = std::collections::HashSet::new();
    let mut count = 0usize;
    loop {
        let key = com_parse_token(&mut state, MAX_LOC_KEY, false);
        if key.is_empty() {
            break;
        }
        let mut equals = com_parse_token(&mut state, MAX_STRING_CHARS, false);
        let mut has_platform_spec = false;
        let mut platform_tags: Vec<String> = Vec::new();
        if equals.is_empty() {
            break;
        } else if equals.starts_with('<') {
            has_platform_spec = true;
            while !equals.is_empty() && !equals.ends_with('>') {
                platform_tags.push(strip_platform_tag(&equals));
                equals = com_parse_token(&mut state, MAX_STRING_CHARS, false);
            }
            if !equals.is_empty() {
                platform_tags.push(strip_platform_tag(&equals));
            }
            equals = com_parse_token(&mut state, MAX_STRING_CHARS, false);
        }
        if equals != "=" {
            break;
        }
        let format = com_parse_token(&mut state, MAX_LOC_FORMAT, true);
        let parsed = match loc_parse(&format) {
            Ok(arguments) => arguments,
            Err(error) => {
                if let Some(log) = log.as_mut() {
                    log.warn(&format!("loc parse error ({key}): {error}"));
                }
                continue;
            }
        };
        if has_platform_spec {
            if let Some(platform) = &platform {
                if platform_tags.iter().any(|tag| tag == platform) {
                    table.insert(
                        key,
                        LocString {
                            format,
                            arguments: parsed,
                        },
                    );
                    count += 1;
                }
            }
            continue;
        }
        if last_wins || !seen.contains(&key) {
            seen.insert(key.clone());
            table.insert(
                key,
                LocString {
                    format,
                    arguments: parsed,
                },
            );
            count += 1;
        }
    }
    count
}

/// Localization profile.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LocalizationProfile {
    /// Q1 rerelease (first duplicate wins).
    #[default]
    Q1Rerelease,
    /// Q2 rerelease (last duplicate wins).
    Q2Rerelease,
}

/// A localization table (`LocalizationTable`).
#[derive(Debug, Clone, Default)]
pub struct LocalizationTable {
    table: BTreeMap<String, LocString>,
    profile: LocalizationProfile,
}

impl LocalizationTable {
    /// New table.
    #[must_use]
    pub const fn new(profile: LocalizationProfile) -> Self {
        Self {
            table: BTreeMap::new(),
            profile,
        }
    }

    /// Profile.
    #[must_use]
    pub const fn profile(&self) -> LocalizationProfile {
        self.profile
    }

    /// Look up a key (`lookup`).
    pub fn lookup(&self, key: &str, args: &[String]) -> Option<String> {
        let name = key.strip_prefix('$').unwrap_or(key);
        if !self.table.contains_key(name) {
            return None;
        }
        Some(self.localize(&format!("${name}"), args, true, MAX_STRING_CHARS as i64, None))
    }

    /// Find a record.
    fn find(&self, base: &str) -> Option<&LocString> {
        self.table.get(base)
    }

    /// Localize a source (`localizeSource`).
    #[allow(clippy::too_many_arguments)]
    fn localize_source<'l>(
        &self,
        base: &str,
        allow_in_place: bool,
        args: &[String],
        output_length: i64,
        log: Option<&mut (dyn LibLog + 'l)>,
    ) -> String {
        let mut log = log;
        let working = base.to_string();
        let record: Option<LocString>;
        if !allow_in_place {
            if !working.starts_with('$') {
                return strlcpy(&working, output_length);
            }
            let key = working[1..].to_string();
            let Some(found) = self.find(&key) else {
                return strlcpy(&key, output_length);
            };
            record = Some(found.clone());
            Self::substitute(&key, record.as_ref(), args, output_length, reborrow_log(&mut log))
        } else if let Some(key) = working.strip_prefix('$') {
            let key = key.to_string();
            let Some(found) = self.find(&key) else {
                return strlcpy(&key, output_length);
            };
            record = Some(found.clone());
            Self::substitute(&key, record.as_ref(), args, output_length, reborrow_log(&mut log))
        } else if loc_has_arguments(&working) {
            let format = strlcpy(&working, MAX_LOC_FORMAT as i64);
            let parsed = match loc_parse(&format) {
                Ok(parsed) => parsed,
                Err(error) => {
                    if let Some(log) = log.as_mut() {
                        log.warn(&format!("in-place localization of \"{working}\" failed: {error}"));
                    }
                    return strlcpy(&working, output_length);
                }
            };
            record = Some(LocString {
                format,
                arguments: parsed,
            });
            Self::substitute(&working, record.as_ref(), args, output_length, reborrow_log(&mut log))
        } else {
            strlcpy(&working, output_length)
        }
    }

    fn substitute<'l>(
        working: &str,
        record: Option<&LocString>,
        args: &[String],
        output_length: i64,
        log: Option<&mut (dyn LibLog + 'l)>,
    ) -> String {
        let mut log = log;
        let Some(record) = record else {
            return strlcpy(working, output_length);
        };
        if record.arguments.is_empty() {
            return strlcpy(&record.format, output_length);
        }
        for arg in &record.arguments {
            if arg.arg_index >= args.len() {
                if let Some(log) = log.as_mut() {
                    log.warn(&format!(
                        "Loc_Localize: base \"{working}\" localized with too few arguments"
                    ));
                }
                return strlcpy(working, output_length);
            }
        }
        // Rust slices cannot hold nulls; the invalid-argument guard is vacuous.
        let Some(first) = record.arguments.first() else {
            return strlcpy(&record.format, output_length);
        };
        let mut output = strnlcpy(&record.format, first.start, output_length);
        // Nested tables are unavailable here; resolve nested args
        // against an empty table (literals pass through).
        let empty = LocalizationTable::new(LocalizationProfile::Q1Rerelease);
        let mut arg = *first;
        for next in record.arguments.iter().skip(1) {
            let localized = empty.localize_source(
                &args[arg.arg_index],
                false,
                &[],
                MAX_STRING_CHARS as i64,
                reborrow_log(&mut log),
            );
            output = strlcat(&output, &localized, output_length);
            let rest: String = record.format.chars().skip(arg.end).collect();
            output = strnlcat(&output, &rest, next.start - arg.end, output_length);
            arg = *next;
        }
        let localized = empty.localize_source(
            &args[arg.arg_index],
            false,
            &[],
            MAX_STRING_CHARS as i64,
            reborrow_log(&mut log),
        );
        output = strlcat(&output, &localized, output_length);
        let rest: String = record.format.chars().skip(arg.end).collect();
        strlcat(&output, &rest, output_length)
    }

    /// Clear the table.
    pub fn clear(&mut self) {
        self.table.clear();
    }

    /// Reload the table (`reload`).
    pub fn reload<'l>(
        &mut self,
        bytes: Option<&[u8]>,
        options: &LocReloadOptions,
        log: Option<&mut (dyn LibLog + 'l)>,
    ) -> usize {
        self.clear();
        let Some(bytes) = bytes else {
            return 0;
        };
        let last_wins = options
            .duplicate_keys
            .map_or(self.profile == LocalizationProfile::Q2Rerelease, |policy| {
                policy == DuplicateKeys::Last
            });
        // First-wins needs a pre-populated seen set; emulate by
        // collecting with merge semantics on a scratch table.
        let mut scratch = BTreeMap::new();
        let mut seen = std::collections::HashSet::new();
        let mut count = 0usize;
        let platform = options.platform.map(str::to_ascii_lowercase);
        let text = String::from_utf8_lossy(bytes).into_owned();
        let mut state = TokenState {
            bytes: text.as_bytes(),
            index: 0,
        };
        let mut log = log;
        loop {
            let key = com_parse_token(&mut state, MAX_LOC_KEY, false);
            if key.is_empty() {
                break;
            }
            let mut equals = com_parse_token(&mut state, MAX_STRING_CHARS, false);
            let mut has_platform_spec = false;
            let mut platform_tags: Vec<String> = Vec::new();
            if equals.is_empty() {
                break;
            } else if equals.starts_with('<') {
                has_platform_spec = true;
                while !equals.is_empty() && !equals.ends_with('>') {
                    platform_tags.push(strip_platform_tag(&equals));
                    equals = com_parse_token(&mut state, MAX_STRING_CHARS, false);
                }
                if !equals.is_empty() {
                    platform_tags.push(strip_platform_tag(&equals));
                }
                equals = com_parse_token(&mut state, MAX_STRING_CHARS, false);
            }
            if equals != "=" {
                break;
            }
            let format = com_parse_token(&mut state, MAX_LOC_FORMAT, true);
            let parsed = match loc_parse(&format) {
                Ok(parsed) => parsed,
                Err(error) => {
                    if let Some(log) = log.as_mut() {
                        log.warn(&format!("loc parse error ({key}): {error}"));
                    }
                    continue;
                }
            };
            if has_platform_spec {
                if let Some(platform) = &platform {
                    if platform_tags.iter().any(|tag| tag == platform) {
                        scratch.insert(
                            key,
                            LocString {
                                format,
                                arguments: parsed,
                            },
                        );
                        count += 1;
                    }
                }
                continue;
            }
            if last_wins || !seen.contains(&key) {
                seen.insert(key.clone());
                scratch.insert(
                    key,
                    LocString {
                        format,
                        arguments: parsed,
                    },
                );
                count += 1;
            }
        }
        self.table = scratch;
        if let Some(log) = log.as_mut() {
            log.info(&format!("Loaded {count} localization strings"));
        }
        count
    }

    /// Merge a file (`merge`).
    pub fn merge<'l>(
        &mut self,
        bytes: Option<&[u8]>,
        options: &LocReloadOptions,
        log: Option<&mut (dyn LibLog + 'l)>,
    ) -> usize {
        let Some(bytes) = bytes else {
            return 0;
        };
        let last_wins = options
            .duplicate_keys
            .map_or(self.profile == LocalizationProfile::Q2Rerelease, |policy| {
                policy == DuplicateKeys::Last
            });
        loc_merge_into(bytes, options, log, &mut self.table, last_wins)
    }

    /// Entry count.
    #[must_use]
    pub fn size(&self) -> usize {
        self.table.len()
    }

    /// Load an ordered tier (`loadOrdered`).
    pub fn load_ordered<'l>(
        &mut self,
        primary: &LocLoadTier,
        fallback: &LocLoadTier,
        options: &LocReloadOptions,
        log: Option<&mut (dyn LibLog + 'l)>,
    ) -> usize {
        let mut log = log;
        let tier = if primary.base.is_some() { primary } else { fallback };
        self.reload(tier.base.as_deref(), options, reborrow_log(&mut log));
        for mods in &tier.mods {
            self.merge(Some(mods), options, reborrow_log(&mut log));
        }
        self.table.len()
    }

    /// Initialize (`init`).
    pub fn init(&mut self, bytes: Option<&[u8]>, options: &LocReloadOptions) -> usize {
        self.reload(bytes, options, None)
    }

    /// Localize (`localize`).
    pub fn localize<'l>(
        &self,
        base: &str,
        args: &[String],
        allow_in_place: bool,
        output_bytes: i64,
        log: Option<&mut (dyn LibLog + 'l)>,
    ) -> String {
        assert!(output_bytes >= 0, "Invalid localization output size");
        self.localize_source(base, allow_in_place, args, output_bytes, log)
    }

    /// Byte-oriented output with raw truncation (`localizeBytes`).
    pub fn localize_bytes(&self, base: &str, args: &[String], allow_in_place: bool, output_bytes: i64) -> Vec<u8> {
        assert!(output_bytes >= 0, "Invalid localization output size");
        let text = self.localize_source(base, allow_in_place, args, i64::MAX, None);
        let bytes = text.as_bytes();
        let end = (output_bytes.max(0) - 1).max(0) as usize;
        bytes[..end.min(bytes.len())].to_vec()
    }
}

/// A load tier (`LocLoadTier`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocLoadTier {
    /// Base file.
    pub base: Option<Vec<u8>>,
    /// Mod overlays.
    pub mods: Vec<Vec<u8>>,
}

/// Language from a locale tag (`Loc_LanguageFromLocale`).
#[must_use]
pub fn loc_language_from_locale(tag: Option<&str>) -> String {
    let Some(tag) = tag else {
        return "english".to_string();
    };
    if tag.is_empty() {
        return "english".to_string();
    }
    let stripped = tag.split('.').next().unwrap_or("").split('@').next().unwrap_or("");
    let primary = stripped.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
    match primary.as_str() {
        "en" => "english",
        "fr" => "french",
        "de" => "german",
        "it" => "italian",
        "ru" => "russian",
        "es" => "spanish",
        _ => "english",
    }
    .to_string()
}

/// A seat-bound catalog (`LocalizationCatalog`).
#[derive(Debug, Clone)]
pub struct LocalizationCatalog {
    /// Table.
    pub table: LocalizationTable,
    /// Seat.
    pub seat: SeatId,
}

impl LocalizationCatalog {
    /// New catalog.
    #[must_use]
    pub const fn new(seat: SeatId, profile: LocalizationProfile) -> Self {
        Self {
            table: LocalizationTable::new(profile),
            seat,
        }
    }

    /// Localize.
    pub fn localize(&self, base: &str, args: &[String]) -> String {
        self.table.localize(base, args, true, MAX_STRING_CHARS as i64, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> LocalizationTable {
        let mut table = LocalizationTable::new(LocalizationProfile::Q1Rerelease);
        table.reload(
            Some(b"GREETING = \"Hello {0}!\"\nPLAIN = \"No args\"\n".as_slice()),
            &LocReloadOptions::default(),
            None,
        );
        table
    }

    #[test]
    fn substitutes_positional() {
        let table = table();
        assert_eq!(
            table.localize("$GREETING", &["World".to_string()], true, 1024, None),
            "Hello World!"
        );
    }

    #[test]
    fn missing_key_returns_key() {
        let table = table();
        assert_eq!(table.localize("$NOPE", &[], true, 1024, None), "NOPE");
    }

    #[test]
    fn in_place_formats() {
        let table = table();
        assert_eq!(
            table.localize("Hi {0}", &["Bob".to_string()], true, 1024, None),
            "Hi Bob"
        );
    }

    #[test]
    fn too_few_args_returns_base() {
        let table = table();
        assert_eq!(table.localize("$GREETING", &[], true, 1024, None), "GREETING");
    }

    #[test]
    fn locales_map() {
        assert_eq!(loc_language_from_locale(Some("fr_FR.UTF-8")), "french");
        assert_eq!(loc_language_from_locale(Some("xx")), "english");
        assert_eq!(loc_language_from_locale(None), "english");
    }

    #[test]
    fn bytes_truncate_raw() {
        let table = table();
        let bytes = table.localize_bytes("$PLAIN", &[], true, 4);
        assert_eq!(bytes, b"No ");
    }

    #[test]
    fn duplicate_policy() {
        let mut first = LocalizationTable::new(LocalizationProfile::Q1Rerelease);
        first.reload(
            Some(b"A = \"1\"\nA = \"2\"\n".as_slice()),
            &LocReloadOptions::default(),
            None,
        );
        assert_eq!(first.localize("$A", &[], true, 1024, None), "1");
        let mut last = LocalizationTable::new(LocalizationProfile::Q2Rerelease);
        last.reload(
            Some(b"A = \"1\"\nA = \"2\"\n".as_slice()),
            &LocReloadOptions::default(),
            None,
        );
        assert_eq!(last.localize("$A", &[], true, 1024, None), "2");
    }

    #[test]
    fn lookup_strips_dollar() {
        let table = table();
        assert_eq!(table.lookup("$PLAIN", &[]).as_deref(), Some("No args"));
        assert!(table.lookup("$MISSING", &[]).is_none());
    }
}
