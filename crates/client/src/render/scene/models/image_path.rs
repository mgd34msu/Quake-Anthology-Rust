//! Model image path normalization (donor
//! `src/render/scene/models/image-path.ts`).

use crate::render::error::RenderError;

/// Normalize an embedded model image name to a content-root-relative path.
///
/// Backslashes become slashes, `.` segments collapse, and `..` segments pop
/// the previous part; escaping the content root or naming a drive is an
/// error.
pub fn model_image_path(name: &str) -> Result<String, RenderError> {
    let rejected = || RenderError::BadWire(format!("invalid model image path: {name}"));
    let path = name.replace('\\', "/");
    if path.contains('\0') {
        return Err(rejected());
    }
    if path.len() >= 2 && path.as_bytes()[0].is_ascii_alphabetic() && path.as_bytes()[1] == b':' {
        return Err(rejected());
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        if part.is_empty() {
            return Err(rejected());
        }
        if part == "." {
            continue;
        }
        if part == ".." {
            if parts.pop().is_none() {
                return Err(RenderError::BadWire(format!(
                    "model image escapes content root: {name}"
                )));
            }
        } else {
            parts.push(part);
        }
    }
    qa_content::paths::normalize_resource_path(&parts.join("/")).map_err(|_| rejected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backslashes_become_slashes() {
        assert_eq!(
            model_image_path("models\\players\\visor\\skin.jpg").expect("path"),
            "models/players/visor/skin.jpg"
        );
    }

    #[test]
    fn dot_segments_collapse() {
        assert_eq!(
            model_image_path("models/./visor/../visor/skin.jpg").expect("path"),
            "models/visor/skin.jpg"
        );
    }

    #[test]
    fn escape_is_rejected() {
        assert!(model_image_path("../secret.lmp").is_err());
        assert!(model_image_path("models/../../x").is_err());
    }

    #[test]
    fn drive_and_empty_segments_rejected() {
        assert!(model_image_path("c:/models/skin.jpg").is_err());
        assert!(model_image_path("models//skin.jpg").is_err());
        assert!(model_image_path("models/skin\0.jpg").is_err());
    }
}
