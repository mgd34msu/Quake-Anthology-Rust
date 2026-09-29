//! Brace-block tokenizer from `src/bots/behavior/rerelease/data/blockparse.ts`.
//!
//! Shared reader for the rerelease `bots/*.txt` family. A file is
//! blank lines and blocks; a block is an optional header plus `{`
//! field lines `}`; a field's value is every token after the key up
//! to the end of that source line. `//` comments strip first.

/// One block field: key plus value tokens.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockField {
    /// First token on the line.
    pub key: String,
    /// Remaining tokens on the line.
    pub values: Vec<String>,
    /// 1-based source line.
    pub line: usize,
}

/// One brace block.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// Header tokens before `{`.
    pub header: Vec<String>,
    /// Fields.
    pub fields: Vec<BlockField>,
    /// 1-based line of `{`.
    pub line: usize,
}

/// Parse error.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockParseError {
    /// Message.
    pub message: String,
    /// 1-based line.
    pub line: usize,
}

/// Parse result.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockParseResult {
    /// Blocks.
    pub blocks: Vec<Block>,
    /// Errors.
    pub errors: Vec<BlockParseError>,
}

/// Parse brace blocks.
#[must_use]
pub fn parse_blocks(text: &str) -> BlockParseResult {
    let mut blocks = Vec::new();
    let mut errors = Vec::new();
    let mut stream: Vec<(String, usize)> = Vec::new();
    for (index, raw_line) in text.lines().enumerate() {
        for token in split_line(strip_comment(raw_line)) {
            stream.push((token, index + 1));
        }
    }
    let mut header: Vec<String> = Vec::new();
    let mut current: Option<Block> = None;
    let mut index = 0;
    while index < stream.len() {
        let line = stream[index].1;
        if stream[index].0 == "{" {
            if current.is_some() {
                errors.push(BlockParseError {
                    message: "nested block".to_owned(),
                    line,
                });
                index += 1;
                continue;
            }
            current = Some(Block {
                header: std::mem::take(&mut header),
                fields: Vec::new(),
                line,
            });
            index += 1;
            continue;
        }
        if stream[index].0 == "}" {
            match current.take() {
                Some(block) => blocks.push(block),
                None => errors.push(BlockParseError {
                    message: "unmatched closing brace".to_owned(),
                    line,
                }),
            }
            header.clear();
            index += 1;
            continue;
        }
        match current.as_mut() {
            Some(block) => {
                let key = stream[index].0.clone();
                let key_line = stream[index].1;
                index += 1;
                let mut values = Vec::new();
                while index < stream.len()
                    && stream[index].1 == key_line
                    && stream[index].0 != "{"
                    && stream[index].0 != "}"
                {
                    values.push(stream[index].0.clone());
                    index += 1;
                }
                block.fields.push(BlockField {
                    key,
                    values,
                    line: key_line,
                });
            }
            None => {
                header.push(stream[index].0.clone());
                index += 1;
            }
        }
    }
    if current.is_some() {
        errors.push(BlockParseError {
            message: "unterminated block".to_owned(),
            line: text.lines().count(),
        });
    }
    BlockParseResult { blocks, errors }
}

fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut in_quotes = false;
    for index in 0..bytes.len() {
        if bytes[index] == b'"' {
            in_quotes = !in_quotes;
        } else if !in_quotes && bytes[index] == b'/' && index + 1 < bytes.len() && bytes[index + 1] == b'/' {
            return &line[..index];
        }
    }
    line
}

fn split_line(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut in_quotes = false;
    for c in line.chars() {
        if c == '"' {
            if in_quotes {
                tokens.push(std::mem::take(&mut token));
                in_quotes = false;
            } else {
                if !token.is_empty() {
                    tokens.push(std::mem::take(&mut token));
                }
                in_quotes = true;
            }
            continue;
        }
        if in_quotes {
            token.push(c);
            continue;
        }
        if c.is_whitespace() {
            if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            }
            continue;
        }
        if (c == '{' || c == '}') && token.is_empty() {
            tokens.push(c.to_string());
            continue;
        }
        token.push(c);
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    tokens
}

/// First value as a string.
#[must_use]
pub fn field_string(values: &[String]) -> Option<String> {
    values.first().cloned()
}

/// First value as a number.
#[must_use]
pub fn field_number(values: &[String]) -> Option<f64> {
    values.first()?.parse::<f64>().ok()
}

/// First value as a bool (`1`/`true`/`yes`).
#[must_use]
pub fn field_bool(values: &[String]) -> Option<bool> {
    match values.first().map(|value| value.to_ascii_lowercase()).as_deref() {
        Some("1" | "true" | "yes") => Some(true),
        Some("0" | "false" | "no") => Some(false),
        _ => None,
    }
}

/// Pipe-separated flag list.
#[must_use]
pub fn field_flag_list(values: &[String]) -> Vec<String> {
    values.iter().filter(|value| value.as_str() != "|").cloned().collect()
}

/// Find a block field by key.
#[must_use]
pub fn block_field<'b>(block: &'b Block, key: &str) -> Option<&'b BlockField> {
    block.fields.iter().find(|field| field.key.eq_ignore_ascii_case(key))
}
