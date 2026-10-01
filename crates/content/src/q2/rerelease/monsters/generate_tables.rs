//! Rerelease monster table generator (`src/content/q2/rerelease/monsters/generate-tables.ts`).
//!
//! Pure string driver ported from the TypeScript build script. It reads
//! rerelease monster sources as strings and emits the generated table
//! sources as strings; all file access stays with the caller.

/// Whether a byte is a word character (`\w`).
fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// Whether text is a bare word (`^\w+$`).
fn is_word_text(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(is_word)
}

/// Strip C and C++ comments (`clean`).
pub fn clean_source(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'/' && index + 1 < bytes.len() && bytes[index + 1] == b'*' {
            index += 2;
            while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
                index += 1;
            }
            index = (index + 2).min(bytes.len());
        } else if bytes[index] == b'/' && index + 1 < bytes.len() && bytes[index + 1] == b'/' {
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
        } else {
            out.push(bytes[index] as char);
            index += 1;
        }
    }
    out
}

/// Split top-level comma parts (`split`).
pub fn split_top_level(source: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (index, char) in source.char_indices() {
        if char == '{' || char == '(' || char == '[' {
            depth += 1;
        }
        if char == '}' || char == ')' || char == ']' {
            depth -= 1;
        }
        if char == ',' && depth == 0 {
            parts.push(source[start..index].trim().to_string());
            start = index + 1;
        }
    }
    let last = source[start..].trim();
    if !last.is_empty() {
        parts.push(last.to_string());
    }
    parts
}

/// Read a braced initializer body (`body`).
pub fn source_body(source: &str, start: usize) -> String {
    let bytes = source.as_bytes();
    let mut depth = 0;
    let mut index = start;
    while index < bytes.len() {
        if bytes[index] == b'{' {
            depth += 1;
        }
        if bytes[index] == b'}' {
            depth -= 1;
            if depth == 0 {
                return source[start + 1..index].to_string();
            }
        }
        index += 1;
    }
    panic!("Unclosed source initializer");
}

/// Whether text is a plain number literal.
fn is_number_literal(text: &str) -> bool {
    let text = text.strip_prefix('-').unwrap_or(text);
    if text.is_empty() {
        return false;
    }
    if let Some((head, tail)) = text.split_once('.') {
        let head_ok = !head.is_empty() && head.bytes().all(|byte| byte.is_ascii_digit());
        let tail_ok = tail.bytes().all(|byte| byte.is_ascii_digit());
        let dotted_ok = text.starts_with('.') && !tail.is_empty() && tail_ok;
        (head_ok || text.starts_with('.')) && tail_ok && (head_ok || dotted_ok)
    } else {
        text.bytes().all(|byte| byte.is_ascii_digit())
    }
}

/// Strip the `f` suffix after a digit or dot.
fn strip_float_suffix(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        let follows_digit = index > 0 && (bytes[index - 1].is_ascii_digit() || bytes[index - 1] == b'.');
        let word_end = index + 1 >= bytes.len() || !is_word(bytes[index + 1]);
        if byte == b'f' && follows_digit && word_end {
            index += 1;
            continue;
        }
        out.push(byte as char);
        index += 1;
    }
    out
}

/// Resolve a source number (`numeric`).
pub fn source_numeric(source: &str, values: &[(String, f64)]) -> f64 {
    let text = strip_float_suffix(source.trim());
    if let Some(found) = values.iter().rev().find(|(name, _)| *name == text) {
        return found.1;
    }
    if is_number_literal(&text) {
        return text.parse::<f64>().expect("source number parses");
    }
    let bytes = text.as_bytes();
    let mut positions = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'+' || byte == b'-' || byte == b'*' {
            positions.push(index);
        }
        index += 1;
    }
    for position in positions {
        let left = text[..position].trim();
        let right = text[position + 1..].trim();
        if left.is_empty() || !is_number_literal(right) {
            continue;
        }
        let value = right.parse::<f64>().expect("source number parses");
        let base = source_numeric(left, values);
        return match bytes[position] {
            b'+' => base + value,
            b'-' => base - value,
            _ => base * value,
        };
    }
    panic!("Unsupported source number {source}");
}

/// Collect frame and flash constants (`constants`).
pub fn source_constants(source: &str) -> Vec<(String, f64)> {
    let mut result: Vec<(String, f64)> = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("#define") else {
            continue;
        };
        if !rest.starts_with(char::is_whitespace) {
            continue;
        }
        let rest = rest.trim_start();
        let end = rest.bytes().take_while(|byte| is_word(*byte)).count();
        let (name, value) = (rest[..end].to_string(), rest[end..].trim().to_string());
        if name.starts_with("FRAME_") || name.starts_with("MODEL_SCALE") {
            let value = source_numeric(&value, &result);
            result.push((name, value));
        }
    }
    let bytes = source.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let Some(found) = source[index..].find("enum") else {
            break;
        };
        let mut cursor = index + found + 4;
        while cursor < bytes.len() && bytes[cursor] != b'{' {
            if bytes[cursor] == b'}' {
                break;
            }
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] != b'{' {
            index = cursor + 1;
            continue;
        }
        let mut end = cursor + 1;
        while end < bytes.len() && bytes[end] != b'}' {
            end += 1;
        }
        let mut next = 0.0;
        for field in split_top_level(&source[cursor + 1..end]) {
            let mut parts = field.splitn(2, '=');
            let name = parts.next().map(str::trim).unwrap_or_default().to_string();
            let value = parts.next().map(str::trim);
            let frame = (name.starts_with("FRAME_") || name.starts_with("MZ2_")) && is_word_text(&name);
            if !frame {
                continue;
            }
            if let Some(value) = value {
                next = source_numeric(value, &result);
            }
            result.push((name, next));
            next += 1.0;
        }
        index = end + 1;
    }
    result
}

/// Encode a JSON string.
fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for char in text.chars() {
        match char {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other if (other as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", other as u32)),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// Format a source number the way the script interpolates it.
fn format_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// Encode frame actions (`actions`).
pub fn source_actions(source: Option<&str>, values: &[(String, f64)]) -> String {
    let Some(source) = source else {
        return "[]".to_string();
    };
    if source == "nullptr" || source == "NULL" {
        return "[]".to_string();
    }
    if is_word_text(source) {
        return format!("[{}]", json_string(source));
    }
    if source.starts_with("[]") {
        let start = source.find('{').expect("frame lambda has a body");
        let mut result = Vec::new();
        for statement in source_body(source, start).split(';') {
            let statement = statement.trim();
            if statement.is_empty() {
                continue;
            }
            if statement.ends_with("(self)") {
                let callback = statement[..statement.len() - 6].to_string();
                if is_word_text(&callback) {
                    result.push(json_string(&callback));
                    continue;
                }
            }
            let marker = "self->monsterinfo.nextframe";
            if let Some(assigned) = statement.strip_prefix(marker) {
                let assigned = assigned.trim_start().strip_prefix('=').map(str::trim);
                if assigned.is_some_and(is_word_text) {
                    let frame = source_numeric(assigned.expect("frame checked"), values);
                    result.push(format!("{{\"kind\":\"nextframe\",\"frame\":{}}}", format_number(frame)));
                    continue;
                }
            }
            panic!("Unsupported source frame statement {statement}");
        }
        return format!("[{}]", result.join(","));
    }
    panic!("Unsupported source frame action {source}");
}

/// Find a frame header include (`#include "m_*.h"`).
fn frame_header(source: &str) -> String {
    let mut index = 0;
    while let Some(found) = source[index..].find("#include \"m_") {
        let start = index + found + "#include \"".len();
        let Some(end) = source[start..].find('"') else {
            break;
        };
        let header = source[start..start + end].to_string();
        if header.ends_with(".h") {
            return header;
        }
        index = start + end + 1;
    }
    panic!("missing frame header");
}

/// Collect `constexpr` numbers into the value table.
fn collect_constexpr(source: &str, values: &mut Vec<(String, f64)>) {
    let mut index = 0;
    while let Some(found) = source[index..].find("constexpr") {
        let mut cursor = index + found + "constexpr".len();
        let bytes = source.as_bytes();
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let mut matched = false;
        for keyword in ["float", "int32_t", "int"] {
            if source[cursor..].starts_with(keyword) {
                let after = cursor + keyword.len();
                if after < bytes.len() && !bytes[after].is_ascii_whitespace() {
                    continue;
                }
                cursor = after;
                matched = true;
                break;
            }
        }
        if !matched {
            index = cursor + 1;
            continue;
        }
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let end = source[cursor..].bytes().take_while(|byte| is_word(*byte)).count();
        let name = source[cursor..cursor + end].to_string();
        cursor += end;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] != b'=' {
            index = cursor + 1;
            continue;
        }
        cursor += 1;
        let Some(semi) = source[cursor..].find(';') else {
            break;
        };
        let value = source[cursor..cursor + semi].trim().to_string();
        let plain = value.strip_suffix('f').unwrap_or(&value);
        if !plain.is_empty()
            && plain
                .bytes()
                .all(|byte| byte == b'-' || byte.is_ascii_digit() || byte == b'.')
        {
            let number = source_numeric(&value, values);
            values.push((name, number));
        }
        index = cursor + semi + 1;
    }
}

/// Find `mframe_t` array matches (`name`, body start).
fn frame_arrays(source: &str) -> Vec<(String, usize)> {
    let mut result = Vec::new();
    let mut index = 0;
    while let Some(found) = source[index..].find("mframe_t") {
        let mut cursor = index + found + "mframe_t".len();
        let bytes = source.as_bytes();
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let end = source[cursor..].bytes().take_while(|byte| is_word(*byte)).count();
        if end == 0 {
            index = cursor + 1;
            continue;
        }
        let name = source[cursor..cursor + end].to_string();
        cursor += end;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] != b'[' {
            index = cursor + 1;
            continue;
        }
        let Some(close) = source[cursor..].find(']') else {
            break;
        };
        cursor += close + 1;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] != b'=' {
            index = cursor + 1;
            continue;
        }
        cursor += 1;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] != b'{' {
            index = cursor + 1;
            continue;
        }
        result.push((name, cursor));
        index = cursor + 1;
    }
    result
}

/// Find `MMOVE_T` move matches (`name`, body start).
fn move_blocks(source: &str) -> Vec<(String, usize)> {
    let mut result = Vec::new();
    let mut index = 0;
    while let Some(found) = source[index..].find("MMOVE_T(") {
        let mut cursor = index + found + "MMOVE_T(".len();
        let end = source[cursor..].bytes().take_while(|byte| is_word(*byte)).count();
        if end == 0 {
            index = cursor + 1;
            continue;
        }
        let name = source[cursor..cursor + end].to_string();
        cursor += end;
        let bytes = source.as_bytes();
        if cursor >= bytes.len() || bytes[cursor] != b')' {
            index = cursor + 1;
            continue;
        }
        cursor += 1;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] != b'=' {
            index = cursor + 1;
            continue;
        }
        cursor += 1;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] != b'{' {
            index = cursor + 1;
            continue;
        }
        result.push((name, cursor));
        index = cursor + 1;
    }
    result
}

/// Generate a species table source (`generate`).
pub fn generate_rerelease_tables(cpp_source: &str, header_source: &str, species: &str) -> String {
    let source = clean_source(cpp_source);
    let header = frame_header(&source);
    let mut values = source_constants(&clean_source(header_source));
    collect_constexpr(&source, &mut values);
    let mut arrays: Vec<(String, Vec<String>)> = Vec::new();
    for (name, start) in frame_arrays(&source) {
        let rows = split_top_level(&source_body(&source, start));
        let mut table = Vec::new();
        for row in rows {
            let start = row.find('{').expect("frame row has a body");
            let fields = split_top_level(&source_body(&row, start));
            let ai = fields.first().cloned().unwrap_or_else(|| "nullptr".to_string());
            let distance = fields.get(1).cloned().unwrap_or_else(|| "0".to_string());
            let callback = fields.get(2).cloned();
            let lerp = fields.get(3).cloned().unwrap_or_else(|| "-1".to_string());
            if !is_word_text(&ai) {
                panic!("Unsupported frame AI {ai}");
            }
            let kind = if ai == "nullptr" || ai == "NULL" {
                "none".to_string()
            } else {
                ai.strip_prefix("ai_").unwrap_or(&ai).to_string()
            };
            let encoded = match kind.as_str() {
                "stand" | "walk" | "run" | "charge" | "move" | "soldier_move" | "turn" | "none" => json_string(&kind),
                _ => format!("{{\"kind\":\"source\",\"name\":{}}}", json_string(&ai)),
            };
            table.push(format!(
                "    {{ ai: {encoded}, distance: Math.fround({}), actions: {}, lerpFrame: {} }},",
                format_number(source_numeric(&distance, &values)),
                source_actions(callback.as_deref(), &values),
                format_number(source_numeric(&lerp, &values)),
            ));
        }
        arrays.push((name, table));
    }
    let mut moves = Vec::new();
    for (name, start) in move_blocks(&source) {
        let fields = split_top_level(&source_body(&source, start));
        let (first, last, frames) = match (fields.first(), fields.get(1), fields.get(2)) {
            (Some(first), Some(last), Some(frames)) => (first, last, frames),
            _ => panic!("Incomplete source move"),
        };
        let table = arrays
            .iter()
            .find(|(candidate, _)| candidate == frames)
            .unwrap_or_else(|| panic!("{name}: unknown frame array {frames}"));
        let start_frame = source_numeric(first, &values);
        let last_frame = source_numeric(last, &values);
        if (table.1.len() as f64) < last_frame - start_frame + 1.0 {
            panic!("{name}: incomplete frame table");
        }
        let scale = fields
            .iter()
            .find(|field| field.starts_with(".sidestep_scale"))
            .and_then(|field| field.split('=').nth(1))
            .unwrap_or("0");
        let end = fields.get(3).cloned().unwrap_or_else(|| "nullptr".to_string());
        moves.push(format!(
            "  {{ name: {}, firstFrame: {}, lastFrame: {}, end: {}, sidestepScale: {}, frames: [\n{}\n  ] }},",
            json_string(&name),
            format_number(start_frame),
            format_number(last_frame),
            if end == "nullptr" {
                "null".to_string()
            } else {
                json_string(&end)
            },
            format_number(source_numeric(scale, &values)),
            table.1.join("\n"),
        ));
    }
    if moves.is_empty() {
        panic!("{species}: no source moves");
    }
    let fields = values
        .iter()
        .filter(|(name, _)| name.starts_with("FRAME_"))
        .map(|(name, value)| format!("  {}: {},", &name[6..], format_number(*value)))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "// Generated from rerelease/m_{species}.cpp and {header}. ZeniMax Media, GPL-2.0.\nimport type {{ MonsterMove }} from \"../../../foundation/monsters/types.ts\";\n\nexport const {species}Frame = {{\n{fields}\n}};\n\nexport const {species}Moves: readonly MonsterMove[] = [\n{}\n];\n",
        moves.join("\n"),
    )
}

/// Generate the muzzle-flash table source.
pub fn generate_rerelease_flashes(game_h_source: &str) -> String {
    let flashes = source_constants(&clean_source(game_h_source))
        .into_iter()
        .filter(|(name, _)| name.starts_with("MZ2_"))
        .map(|(name, value)| format!("  {}: {},", &name[4..], format_number(value)))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "// Original rerelease/game.h muzzle-flash enum. ZeniMax Media, GPL-2.0.\nexport const rereleaseFlash = {{\n{flashes}\n}};\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_strips_comments() {
        assert_eq!(clean_source("a /* x */ b // y\nc"), "a  b \nc");
    }

    #[test]
    fn split_respects_nesting() {
        assert_eq!(
            split_top_level("ai_stand, 0, [](auto self) { walk(self); }, -1"),
            vec![
                "ai_stand".to_string(),
                "0".to_string(),
                "[](auto self) { walk(self); }".to_string(),
                "-1".to_string()
            ],
        );
    }

    #[test]
    fn body_reads_braces() {
        assert_eq!(source_body("f({ a }, { b })", 2), " a ");
    }

    #[test]
    fn numeric_resolves_values() {
        let values = vec![("FRAME_a".to_string(), 3.0)];
        assert_eq!(source_numeric("FRAME_a", &values), 3.0);
        assert_eq!(source_numeric("2.5f", &values), 2.5);
        assert_eq!(source_numeric("FRAME_a+2", &values), 5.0);
        assert_eq!(source_numeric("FRAME_a*2", &values), 6.0);
        assert_eq!(source_numeric("-1", &values), -1.0);
    }

    #[test]
    fn constants_cover_defines_and_enums() {
        let source = "#define FRAME_a 4\n#define OTHER 9\nenum flash { MZ2_ONE, MZ2_TWO = 7, MZ2_THREE };";
        assert_eq!(
            source_constants(source),
            vec![
                ("FRAME_a".to_string(), 4.0),
                ("MZ2_ONE".to_string(), 0.0),
                ("MZ2_TWO".to_string(), 7.0),
                ("MZ2_THREE".to_string(), 8.0),
            ],
        );
    }

    #[test]
    fn actions_encode_callbacks() {
        let values = vec![("FRAME_b".to_string(), 9.0)];
        assert_eq!(source_actions(None, &values), "[]");
        assert_eq!(source_actions(Some("nullptr"), &values), "[]");
        assert_eq!(source_actions(Some("fire"), &values), "[\"fire\"]");
        assert_eq!(
            source_actions(
                Some("[](auto self) { fire(self); self->monsterinfo.nextframe = FRAME_b; }"),
                &values,
            ),
            "[\"fire\",{\"kind\":\"nextframe\",\"frame\":9}]",
        );
    }

    #[test]
    fn generate_emits_species_table() {
        let header = "#define FRAME_stand01 0\n#define FRAME_stand02 1\n";
        let cpp = "#include \"m_species.h\"\nconstexpr float SCALE = 1.5f;\n\
            mframe_t species_frames[] = { { ai_stand, 0, nullptr, -1 }, { ai_walk, 4, fire, FRAME_stand02 } };\n\
            MMOVE_T(species_stand) = { FRAME_stand01, FRAME_stand02, species_frames, nullptr };";
        let output = generate_rerelease_tables(cpp, header, "species");
        assert!(output.contains("export const speciesFrame = {"), "{output}");
        assert!(output.contains("stand01: 0,"), "{output}");
        assert!(
            output.contains("{ name: \"species_stand\", firstFrame: 0, lastFrame: 1"),
            "{output}"
        );
        assert!(
            output.contains("{ ai: \"walk\", distance: Math.fround(4), actions: [\"fire\"], lerpFrame: 1 },"),
            "{output}",
        );
    }

    #[test]
    fn generate_emits_flashes() {
        let output = generate_rerelease_flashes("enum flash { MZ2_A, MZ2_B = 5 };");
        assert!(output.contains("export const rereleaseFlash = {"), "{output}");
        assert!(output.contains("A: 0,"), "{output}");
        assert!(output.contains("B: 5,"), "{output}");
    }
}
