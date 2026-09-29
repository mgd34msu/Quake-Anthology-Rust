//! Bot structure reader from `src/bots/behavior/library/structure.ts`
//! (`l_struct.c`: `ReadStructure`).
//!
//! Reads `{ key value ... }` definition blocks from bot scripts
//! (`.c`, `.w`, `.i` files). Fields carry a name, an optional structure
//! id for nested blocks, and string or numeric values.

use crate::error::BotsError;

/// One parsed structure field.
#[derive(Debug, Clone, PartialEq)]
pub struct StructureField {
    /// Field name.
    pub name: String,
    /// Nested structure id, when the field opens a block.
    pub structure_id: Option<i32>,
    /// String value, when the field is a string.
    pub string: Option<String>,
    /// Numeric value, when the field is numeric.
    pub number: Option<f64>,
}

/// One parsed `{ ... }` definition.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StructureDefinition {
    /// Definition type name (token before the block, when present).
    pub type_name: Option<String>,
    /// Parsed fields.
    pub fields: Vec<StructureField>,
}

impl StructureDefinition {
    /// Find a field by name.
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&StructureField> {
        self.fields.iter().find(|field| field.name == name)
    }

    /// String value of a field, or `None`.
    #[must_use]
    pub fn string(&self, name: &str) -> Option<&str> {
        self.field(name).and_then(|field| field.string.as_deref())
    }

    /// Numeric value of a field, or `None`.
    #[must_use]
    pub fn number(&self, name: &str) -> Option<f64> {
        self.field(name).and_then(|field| field.number)
    }
}

/// Tokenize bot script text: quoted strings stay whole, braces and
/// semicolons split, `//` and `/* */` comments are dropped.
#[must_use]
pub fn tokenize_bot_script(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte.is_ascii_whitespace() || byte == b';' || byte == b',' {
            index += 1;
            continue;
        }
        if byte == b'/' && index + 1 < bytes.len() && bytes[index + 1] == b'/' {
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            continue;
        }
        if byte == b'/' && index + 1 < bytes.len() && bytes[index + 1] == b'*' {
            index += 2;
            while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
                index += 1;
            }
            index = (index + 2).min(bytes.len());
            continue;
        }
        if byte == b'"' {
            let mut token = String::new();
            index += 1;
            while index < bytes.len() && bytes[index] != b'"' {
                if bytes[index] == b'\\' && index + 1 < bytes.len() {
                    index += 1;
                    token.push(bytes[index] as char);
                } else {
                    token.push(bytes[index] as char);
                }
                index += 1;
            }
            index = (index + 1).min(bytes.len());
            tokens.push(token);
            continue;
        }
        if byte == b'{' || byte == b'}' {
            tokens.push((byte as char).to_string());
            index += 1;
            continue;
        }
        let mut token = String::new();
        while index < bytes.len()
            && !bytes[index].is_ascii_whitespace()
            && !matches!(bytes[index], b'{' | b'}' | b';' | b',' | b'"')
        {
            if bytes[index] == b'/' && index + 1 < bytes.len() && matches!(bytes[index + 1], b'/' | b'*') {
                break;
            }
            token.push(bytes[index] as char);
            index += 1;
        }
        if !token.is_empty() {
            tokens.push(token);
        }
    }
    tokens
}

/// Parse top-level `{ ... }` definitions from bot script text.
pub fn read_structure_definitions(text: &str) -> Result<Vec<StructureDefinition>, BotsError> {
    let tokens = tokenize_bot_script(text);
    let mut definitions = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let mut type_name = None;
        if index + 1 < tokens.len() && tokens[index + 1] == "{" {
            type_name = Some(tokens[index].clone());
            index += 1;
        }
        if tokens[index] != "{" {
            return Err(BotsError::BotScript(format!(
                "expected '{{' at token {index}, found '{}'",
                tokens[index]
            )));
        }
        index += 1;
        let mut fields = Vec::new();
        while index < tokens.len() && tokens[index] != "}" {
            let name = tokens[index].clone();
            index += 1;
            if index < tokens.len() && tokens[index] == "{" {
                let (nested, next) = read_nested(&tokens, index)?;
                index = next;
                for (nested_index, nested_field) in nested.iter().enumerate() {
                    fields.push(StructureField {
                        name: format!("{name}.{}", nested_field.name),
                        structure_id: Some(nested_index as i32),
                        string: nested_field.string.clone(),
                        number: nested_field.number,
                    });
                }
            } else if index < tokens.len() {
                let value = tokens[index].clone();
                index += 1;
                let number = value.parse::<f64>().ok();
                fields.push(StructureField {
                    name,
                    structure_id: None,
                    string: if number.is_none() { Some(value) } else { None },
                    number,
                });
            } else {
                return Err(BotsError::BotScript("unterminated structure field".to_owned()));
            }
        }
        if index >= tokens.len() {
            return Err(BotsError::BotScript("unterminated structure block".to_owned()));
        }
        index += 1;
        definitions.push(StructureDefinition { type_name, fields });
    }
    Ok(definitions)
}

fn read_nested(tokens: &[String], open: usize) -> Result<(Vec<StructureField>, usize), BotsError> {
    let mut fields = Vec::new();
    let mut index = open + 1;
    while index < tokens.len() && tokens[index] != "}" {
        let name = tokens[index].clone();
        index += 1;
        if index >= tokens.len() {
            return Err(BotsError::BotScript("unterminated nested block".to_owned()));
        }
        if tokens[index] == "{" {
            let (nested, next) = read_nested(tokens, index)?;
            index = next;
            for (nested_index, nested_field) in nested.iter().enumerate() {
                fields.push(StructureField {
                    name: format!("{name}.{}", nested_field.name),
                    structure_id: Some(nested_index as i32),
                    string: nested_field.string.clone(),
                    number: nested_field.number,
                });
            }
        } else {
            let value = tokens[index].clone();
            index += 1;
            let number = value.parse::<f64>().ok();
            fields.push(StructureField {
                name,
                structure_id: None,
                string: if number.is_none() { Some(value) } else { None },
                number,
            });
        }
    }
    if index >= tokens.len() {
        return Err(BotsError::BotScript("unterminated nested block".to_owned()));
    }
    Ok((fields, index + 1))
}

/// Structure reader over prepared source files.
#[derive(Debug, Default)]
pub struct StructureReader {
    definitions: Vec<StructureDefinition>,
}

impl StructureReader {
    /// Empty reader.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse and append definitions from script text.
    pub fn load(&mut self, text: &str) -> Result<usize, BotsError> {
        let definitions = read_structure_definitions(text)?;
        let count = definitions.len();
        self.definitions.extend(definitions);
        Ok(count)
    }

    /// Parsed definitions.
    #[must_use]
    pub fn definitions(&self) -> &[StructureDefinition] {
        &self.definitions
    }

    /// Definitions with a type name.
    pub fn definitions_of_type(&self, type_name: &str) -> Vec<&StructureDefinition> {
        self.definitions
            .iter()
            .filter(|definition| definition.type_name.as_deref() == Some(type_name))
            .collect()
    }

    /// Clear all definitions.
    pub fn clear(&mut self) {
        self.definitions.clear();
    }
}
