//! Model attachment and grip declarations.
//!
//! Donor: `src/content/model-attachment.ts`. This module also hosts the
//! strict JSON declaration parser shared with [`crate::held_weapon`]: the
//! workspace has no JSON dependency, so declaration files (`JSON.parse` over
//! fatal UTF-8 in the donor) decode through [`parse_declaration_json`] into
//! [`SaveJson`].

use qa_core::math::{cross3, dot3, Vec3};
use thiserror::Error;

use crate::contract::{
    is_content_digest, ContentDigest, ModelAttachmentDefinition, ModelAttachmentTarget, ModelTransform,
};
use crate::value::{SaveJson, SaveReader, ValueError};

/// Model attachment failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ModelAttachmentError {
    /// Malformed declaration (carries the detail).
    #[error("{0}")]
    BadSave(String),
}

impl From<ValueError> for ModelAttachmentError {
    fn from(error: ValueError) -> Self {
        Self::BadSave(error.to_string())
    }
}

#[allow(clippy::cast_possible_truncation)]
fn read_vector(reader: SaveReader) -> Result<Vec3, ModelAttachmentError> {
    Ok(Vec3 {
        x: reader.field("x").finite()? as f32,
        y: reader.field("y").finite()? as f32,
        z: reader.field("z").finite()? as f32,
    })
}

/// Read a grip transform (`readModelGrip`).
pub fn read_model_grip(grip: &SaveReader) -> Result<ModelTransform, ModelAttachmentError> {
    let axis: Vec<Vec3> = grip.field("axis").list(read_vector)?;
    let [first, second, third] = axis.as_slice() else {
        return Err(ModelAttachmentError::BadSave(
            "Model grip requires three source axes".to_string(),
        ));
    };
    if axis.iter().any(|value| (dot3(*value, *value) - 1.0).abs() > 0.001)
        || dot3(*first, *second).abs() > 0.001
        || dot3(*first, *third).abs() > 0.001
        || dot3(*second, *third).abs() > 0.001
        || dot3(cross3(*first, *second), *third) < 0.999
    {
        return Err(ModelAttachmentError::BadSave(
            "Model grip axes must form a rotation".to_string(),
        ));
    }
    let scale_field = grip.field("scale");
    let scale = if scale_field.is_missing() {
        Vec3 { x: 1.0, y: 1.0, z: 1.0 }
    } else {
        read_vector(scale_field)?
    };
    if scale.x == 0.0 || scale.y == 0.0 || scale.z == 0.0 {
        return Err(grip.fail("Model grip scale must be invertible").into());
    }
    Ok(ModelTransform {
        origin: read_vector(grip.field("origin"))?,
        axis: [*first, *second, *third],
        scale,
    })
}

/// Read a model attachment declaration file (`readModelAttachment`).
pub fn read_model_attachment(bytes: &[u8]) -> Result<ModelAttachmentDefinition, ModelAttachmentError> {
    let value = parse_declaration_json(bytes)?;
    let reader = SaveReader::new(&value);
    reader.field("version").literal_i64(1)?;
    let digest = reader.field("digest").string()?;
    if !is_content_digest(&digest) {
        return Err(ModelAttachmentError::BadSave(
            "Model attachment requires its source digest".to_string(),
        ));
    }
    let grip = read_model_grip(&reader.field("grip"))?;
    if reader.field("kind").choice_str(&["joint", "mesh"])? == "joint" {
        return Ok(ModelAttachmentDefinition {
            digest: ContentDigest(digest),
            grip,
            target: ModelAttachmentTarget::Joint {
                name: reader.field("name").string()?,
            },
        });
    }
    let vertices: Vec<i64> = reader.field("vertices").list(|value| value.integer(0))?;
    let [a, b, c] = vertices.as_slice() else {
        return Err(ModelAttachmentError::BadSave(
            "Model attachment requires three distinct source vertices".to_string(),
        ));
    };
    if a == b || a == c || b == c {
        return Err(ModelAttachmentError::BadSave(
            "Model attachment requires three distinct source vertices".to_string(),
        ));
    }
    // The donor carries the three source vertex indices as `[a, b, c]`;
    // the contract stores index triples, so the single triple is preserved
    // as one entry.
    #[allow(clippy::cast_precision_loss)]
    let mesh = (
        reader.field("referenceFrame").integer(0)? as f64,
        [*a as f64, *b as f64, *c as f64],
    );
    Ok(ModelAttachmentDefinition {
        digest: ContentDigest(digest),
        grip,
        target: ModelAttachmentTarget::Mesh {
            reference_frame: mesh.0,
            vertices: vec![mesh.1],
        },
    })
}

/// Strict JSON declaration parser shared with [`crate::held_weapon`].
///
/// Matches `JSON.parse`: numbers decode as `f64` (`1e999` becomes infinity),
/// duplicate object keys are preserved in order with last-wins lookup, and
/// trailing bytes fail. Lone surrogates are rejected (Rust strings cannot
/// hold the donor's WTF-8 output).
pub(crate) fn parse_declaration_json(bytes: &[u8]) -> Result<SaveJson, ModelAttachmentError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| ModelAttachmentError::BadSave("Declaration bytes are not valid UTF-8".to_string()))?;
    let mut parser = JsonParser {
        text: text.as_bytes(),
        cursor: 0,
    };
    let value = parser.parse_value()?;
    parser.skip_whitespace();
    if parser.cursor != parser.text.len() {
        return Err(parser.error("unexpected trailing bytes"));
    }
    Ok(value)
}

struct JsonParser<'a> {
    text: &'a [u8],
    cursor: usize,
}

impl JsonParser<'_> {
    fn error(&self, message: &str) -> ModelAttachmentError {
        ModelAttachmentError::BadSave(format!("declaration:{}: {message}", self.cursor))
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.text.get(self.cursor), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.cursor += 1;
        }
    }

    fn parse_value(&mut self) -> Result<SaveJson, ModelAttachmentError> {
        self.skip_whitespace();
        match self.text.get(self.cursor) {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => Ok(SaveJson::String(self.parse_string()?)),
            Some(b't') => self.parse_literal("true", SaveJson::Bool(true)),
            Some(b'f') => self.parse_literal("false", SaveJson::Bool(false)),
            Some(b'n') => self.parse_literal("null", SaveJson::Null),
            Some(b'-' | b'0'..=b'9') => self.parse_number(),
            _ => Err(self.error("expected a value")),
        }
    }

    fn parse_literal(&mut self, literal: &str, value: SaveJson) -> Result<SaveJson, ModelAttachmentError> {
        if self.text[self.cursor..].starts_with(literal.as_bytes()) {
            self.cursor += literal.len();
            Ok(value)
        } else {
            Err(self.error("expected a value"))
        }
    }

    fn parse_number(&mut self) -> Result<SaveJson, ModelAttachmentError> {
        let start = self.cursor;
        if self.text.get(self.cursor) == Some(&b'-') {
            self.cursor += 1;
        }
        match self.text.get(self.cursor) {
            Some(b'0') => {
                self.cursor += 1;
            }
            Some(b'1'..=b'9') => {
                while matches!(self.text.get(self.cursor), Some(b'0'..=b'9')) {
                    self.cursor += 1;
                }
            }
            _ => return Err(self.error("expected a number")),
        }
        if self.text.get(self.cursor) == Some(&b'.') {
            self.cursor += 1;
            if !matches!(self.text.get(self.cursor), Some(b'0'..=b'9')) {
                return Err(self.error("expected a number"));
            }
            while matches!(self.text.get(self.cursor), Some(b'0'..=b'9')) {
                self.cursor += 1;
            }
        }
        if matches!(self.text.get(self.cursor), Some(b'e' | b'E')) {
            self.cursor += 1;
            if matches!(self.text.get(self.cursor), Some(b'+' | b'-')) {
                self.cursor += 1;
            }
            if !matches!(self.text.get(self.cursor), Some(b'0'..=b'9')) {
                return Err(self.error("expected a number"));
            }
            while matches!(self.text.get(self.cursor), Some(b'0'..=b'9')) {
                self.cursor += 1;
            }
        }
        let token = std::str::from_utf8(&self.text[start..self.cursor]).map_err(|_| self.error("expected a number"))?;
        token
            .parse::<f64>()
            .map(SaveJson::Number)
            .map_err(|_| self.error("expected a number"))
    }

    fn parse_hex4(&mut self) -> Result<u32, ModelAttachmentError> {
        // The cursor sits on the `u` of the escape.
        self.cursor += 1;
        let digits = self
            .text
            .get(self.cursor..self.cursor + 4)
            .ok_or_else(|| self.error("bad unicode escape"))?;
        let mut value = 0u32;
        for digit in digits {
            value = value * 16
                + (*digit as char)
                    .to_digit(16)
                    .ok_or_else(|| self.error("bad unicode escape"))?;
        }
        self.cursor += 4;
        Ok(value)
    }

    fn parse_string(&mut self) -> Result<String, ModelAttachmentError> {
        // The cursor sits on the opening quote.
        self.cursor += 1;
        let mut output = String::new();
        loop {
            match self.text.get(self.cursor) {
                None => return Err(self.error("unterminated string")),
                Some(byte) if *byte < 0x20 => return Err(self.error("unescaped control in string")),
                Some(b'"') => {
                    self.cursor += 1;
                    return Ok(output);
                }
                Some(b'\\') => {
                    self.cursor += 1;
                    match self.text.get(self.cursor) {
                        Some(b'"') => {
                            output.push('"');
                            self.cursor += 1;
                        }
                        Some(b'\\') => {
                            output.push('\\');
                            self.cursor += 1;
                        }
                        Some(b'/') => {
                            output.push('/');
                            self.cursor += 1;
                        }
                        Some(b'b') => {
                            output.push('\u{0008}');
                            self.cursor += 1;
                        }
                        Some(b'f') => {
                            output.push('\u{000C}');
                            self.cursor += 1;
                        }
                        Some(b'n') => {
                            output.push('\n');
                            self.cursor += 1;
                        }
                        Some(b'r') => {
                            output.push('\r');
                            self.cursor += 1;
                        }
                        Some(b't') => {
                            output.push('\t');
                            self.cursor += 1;
                        }
                        Some(b'u') => {
                            let high = self.parse_hex4()?;
                            if (0xD800..0xDC00).contains(&high) {
                                let pair = self.text.get(self.cursor) == Some(&b'\\')
                                    && self.text.get(self.cursor + 1) == Some(&b'u');
                                if !pair {
                                    return Err(self.error("lone surrogate in string"));
                                }
                                self.cursor += 1;
                                let low = self.parse_hex4()?;
                                if !(0xDC00..0xE000).contains(&low) {
                                    return Err(self.error("lone surrogate in string"));
                                }
                                let scalar = 0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00);
                                output.push(
                                    char::from_u32(scalar).ok_or_else(|| self.error("lone surrogate in string"))?,
                                );
                            } else if (0xDC00..0xE000).contains(&high) {
                                return Err(self.error("lone surrogate in string"));
                            } else {
                                output
                                    .push(char::from_u32(high).ok_or_else(|| self.error("lone surrogate in string"))?);
                            }
                        }
                        _ => return Err(self.error("bad string escape")),
                    }
                }
                Some(_) => {
                    let rest =
                        std::str::from_utf8(&self.text[self.cursor..]).map_err(|_| self.error("bad string bytes"))?;
                    let character = rest.chars().next().ok_or_else(|| self.error("unterminated string"))?;
                    output.push(character);
                    self.cursor += character.len_utf8();
                }
            }
        }
    }

    fn parse_object(&mut self) -> Result<SaveJson, ModelAttachmentError> {
        // The cursor sits on the opening brace.
        self.cursor += 1;
        let mut members = Vec::new();
        self.skip_whitespace();
        if self.text.get(self.cursor) == Some(&b'}') {
            self.cursor += 1;
            return Ok(SaveJson::Object(members));
        }
        loop {
            self.skip_whitespace();
            if self.text.get(self.cursor) != Some(&b'"') {
                return Err(self.error("expected an object key"));
            }
            let key = self.parse_string()?;
            self.skip_whitespace();
            if self.text.get(self.cursor) != Some(&b':') {
                return Err(self.error("expected ':'"));
            }
            self.cursor += 1;
            let value = self.parse_value()?;
            members.push((key, value));
            self.skip_whitespace();
            match self.text.get(self.cursor) {
                Some(b',') => {
                    self.cursor += 1;
                }
                Some(b'}') => {
                    self.cursor += 1;
                    return Ok(SaveJson::Object(members));
                }
                _ => return Err(self.error("expected ',' or '}'")),
            }
        }
    }

    fn parse_array(&mut self) -> Result<SaveJson, ModelAttachmentError> {
        // The cursor sits on the opening bracket.
        self.cursor += 1;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.text.get(self.cursor) == Some(&b']') {
            self.cursor += 1;
            return Ok(SaveJson::Array(items));
        }
        loop {
            items.push(self.parse_value()?);
            self.skip_whitespace();
            match self.text.get(self.cursor) {
                Some(b',') => {
                    self.cursor += 1;
                }
                Some(b']') => {
                    self.cursor += 1;
                    return Ok(SaveJson::Array(items));
                }
                _ => return Err(self.error("expected ',' or ']'")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{arr, boolean, num, obj, str};

    fn vector(x: f64, y: f64, z: f64) -> SaveJson {
        obj(vec![("x", num(x)), ("y", num(y)), ("z", num(z))])
    }

    fn grip(axis: SaveJson, origin: SaveJson, scale: Option<SaveJson>) -> SaveJson {
        let mut members = vec![("axis", axis), ("origin", origin)];
        if let Some(scale) = scale {
            members.push(("scale", scale));
        }
        obj(members)
    }

    fn identity_axis() -> SaveJson {
        arr(vec![
            vector(1.0, 0.0, 0.0),
            vector(0.0, 1.0, 0.0),
            vector(0.0, 0.0, 1.0),
        ])
    }

    #[test]
    fn grip_reads_identity_with_default_scale() {
        let value = grip(identity_axis(), vector(1.0, 2.0, 3.0), None);
        let grip = read_model_grip(&SaveReader::new(&value)).unwrap();
        assert_eq!(grip.origin, Vec3 { x: 1.0, y: 2.0, z: 3.0 });
        assert_eq!(grip.scale, Vec3 { x: 1.0, y: 1.0, z: 1.0 });
        assert_eq!(grip.axis[0], Vec3 { x: 1.0, y: 0.0, z: 0.0 });
    }

    #[test]
    fn grip_rejects_non_rotations_bad_scales_and_counts() {
        let origin = vector(0.0, 0.0, 0.0);
        // Scaled axis.
        let scaled = grip(
            arr(vec![
                vector(2.0, 0.0, 0.0),
                vector(0.0, 1.0, 0.0),
                vector(0.0, 0.0, 1.0),
            ]),
            origin.clone(),
            None,
        );
        assert!(read_model_grip(&SaveReader::new(&scaled)).is_err());
        // Non-orthogonal axes.
        let skewed = grip(
            arr(vec![
                vector(1.0, 0.0, 0.0),
                vector(1.0, 0.0, 0.0),
                vector(0.0, 0.0, 1.0),
            ]),
            origin.clone(),
            None,
        );
        let error = read_model_grip(&SaveReader::new(&skewed)).unwrap_err();
        assert_eq!(error.to_string(), "Model grip axes must form a rotation");
        // Left-handed axes.
        let mirrored = grip(
            arr(vec![
                vector(1.0, 0.0, 0.0),
                vector(0.0, 1.0, 0.0),
                vector(0.0, 0.0, -1.0),
            ]),
            origin.clone(),
            None,
        );
        assert!(read_model_grip(&SaveReader::new(&mirrored)).is_err());
        // Zero scale carries the reader path.
        let flat = grip(identity_axis(), origin.clone(), Some(vector(1.0, 0.0, 1.0)));
        let error = read_model_grip(&SaveReader::new(&flat)).unwrap_err();
        assert!(
            error.to_string().contains("Model grip scale must be invertible"),
            "{error}"
        );
        // Wrong axis count.
        let short = grip(arr(vec![vector(1.0, 0.0, 0.0)]), origin, None);
        assert!(read_model_grip(&SaveReader::new(&short)).is_err());
    }

    fn digest() -> String {
        format!("sha256:{}", "ab".repeat(32))
    }

    #[test]
    fn attachment_reads_joint_and_mesh_files() {
        let joint = format!(
            r#"{{"version":1,"digest":"{}","grip":{{"axis":[{{"x":1,"y":0,"z":0}},{{"x":0,"y":1,"z":0}},{{"x":0,"y":0,"z":1}}],"origin":{{"x":0,"y":0,"z":0}}}},"kind":"joint","name":"tag_weapon"}}"#,
            digest()
        );
        let parsed = read_model_attachment(joint.as_bytes()).unwrap();
        assert_eq!(parsed.digest.as_str(), digest());
        assert!(matches!(parsed.target, ModelAttachmentTarget::Joint { .. }));
        let mesh = format!(
            r#"{{"version":1,"digest":"{}","grip":{{"axis":[{{"x":1,"y":0,"z":0}},{{"x":0,"y":1,"z":0}},{{"x":0,"y":0,"z":1}}],"origin":{{"x":0,"y":0,"z":0}}}},"kind":"mesh","referenceFrame":3,"vertices":[4,9,12]}}"#,
            digest()
        );
        let parsed = read_model_attachment(mesh.as_bytes()).unwrap();
        match &parsed.target {
            ModelAttachmentTarget::Mesh {
                reference_frame,
                vertices,
            } => {
                assert_eq!(*reference_frame, 3.0);
                assert_eq!(vertices, &vec![[4.0, 9.0, 12.0]]);
            }
            ModelAttachmentTarget::Joint { .. } => panic!("expected mesh target"),
        }
        // Duplicate and miscounted vertices fail.
        let duplicate = mesh.replace("[4,9,12]", "[4,4,12]");
        let error = read_model_attachment(duplicate.as_bytes()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Model attachment requires three distinct source vertices"
        );
        let short = mesh.replace("[4,9,12]", "[4,9]");
        assert!(read_model_attachment(short.as_bytes()).is_err());
        // Bad digest and version fail.
        assert!(read_model_attachment(
            br#"{"version":2,"digest":"x","grip":{"axis":[],"origin":{"x":0,"y":0,"z":0}},"kind":"joint","name":"n"}"#
        )
        .is_err());
        let bad_digest = joint.replace(&digest(), "nope");
        let error = read_model_attachment(bad_digest.as_bytes()).unwrap_err();
        assert_eq!(error.to_string(), "Model attachment requires its source digest");
    }

    #[test]
    fn declaration_json_matches_parse_semantics() {
        let text = r#"{"s":"a\nbé𝄞","n":-12.5e2,"t":true,"f":false,"z":null,"a":[1,{"k":"v"}],"e":"\u0041"}"#;
        let value = parse_declaration_json(text.as_bytes()).unwrap();
        let reader = SaveReader::new(&value);
        assert_eq!(reader.field("s").string().unwrap(), "a\nbé𝄞");
        assert_eq!(reader.field("n").number().unwrap(), -1250.0);
        assert!(reader.field("t").boolean().unwrap());
        assert!(!reader.field("f").boolean().unwrap());
        assert_eq!(reader.field("e").string().unwrap(), "A");
        // Surrogate pairs combine; lone surrogates fail.
        let pair = parse_declaration_json(br#""\uD834\uDD1E""#).unwrap();
        assert_eq!(pair, str("𝄞"));
        assert!(parse_declaration_json(br#""\uD834x""#).is_err());
        assert!(parse_declaration_json(br#""\uDD1E""#).is_err());
        // Strict grammar: trailing bytes, bad escapes, and bad numbers fail.
        assert!(parse_declaration_json(b"{} trailing").is_err());
        assert!(parse_declaration_json(b"{bad}").is_err());
        assert!(parse_declaration_json(br#"{"a":01}"#).is_err());
        assert!(parse_declaration_json(br#"{"a":1,}"#).is_err());
        assert!(parse_declaration_json(br#""\x""#).is_err());
        assert!(parse_declaration_json(b"").is_err());
        assert!(parse_declaration_json(&[0xff, 0xfe]).is_err());
        // Duplicate keys keep last-wins lookup.
        let duplicate = parse_declaration_json(br#"{"a":1,"a":2}"#).unwrap();
        assert_eq!(SaveReader::new(&duplicate).field("a").integer(0).unwrap(), 2);
        // Builders cover every SaveJson shape used above.
        let _ = arr(vec![boolean(true), num(1.0), str("x")]);
    }
}
