//! Item icon declarations.
//!
//! Donor: `src/content/item-icon.ts`.

use thiserror::Error;

use crate::contract::{ContentId, ResourceRequest, SourceItemIconDeclaration, WeaponHudIcon};
use crate::paths::{normalize_resource_path, PathError};
use crate::value::{SaveReader, ValueError};

/// Item icon failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ItemIconError {
    /// Malformed declaration (carries the detail).
    #[error("{0}")]
    BadSave(String),
    /// Invalid resource path.
    #[error("{0}")]
    BadPath(String),
}

impl From<ValueError> for ItemIconError {
    fn from(error: ValueError) -> Self {
        Self::BadSave(error.to_string())
    }
}

impl From<PathError> for ItemIconError {
    fn from(error: PathError) -> Self {
        Self::BadPath(error.to_string())
    }
}

fn read_lump(reader: &SaveReader, lump: &str) -> Result<String, ItemIconError> {
    if lump.is_empty() || lump.encode_utf16().count() > 16 || lump.contains('\0') {
        return Err(reader.fail("Item icon requires a valid WAD lump name").into());
    }
    Ok(lump.to_string())
}

/// Read an item icon declaration (`readItemIconDeclaration`).
pub fn read_item_icon_declaration(reader: &SaveReader) -> Result<SourceItemIconDeclaration, ItemIconError> {
    let kind = reader.field("kind").choice_str(&["image", "wad-picture", "shader"])?;
    if kind == "shader" {
        return Ok(SourceItemIconDeclaration::Shader {
            name: normalize_resource_path(&reader.field("name").string()?)?,
        });
    }
    let path = normalize_resource_path(&reader.field("path").string()?)?;
    if kind == "image" {
        return Ok(SourceItemIconDeclaration::Image { path });
    }
    Ok(SourceItemIconDeclaration::WadPicture {
        path,
        lump: read_lump(reader, &reader.field("lump").string()?)?,
    })
}

/// Resolve an icon against its content (`resolveItemIcon`).
#[must_use]
pub fn resolve_item_icon(icon: &SourceItemIconDeclaration, content: ContentId) -> WeaponHudIcon {
    match icon {
        SourceItemIconDeclaration::Shader { name } => WeaponHudIcon::Shader {
            content,
            name: name.clone(),
        },
        SourceItemIconDeclaration::Image { path } => WeaponHudIcon::Image {
            resource: ResourceRequest {
                content,
                path: path.clone(),
            },
        },
        SourceItemIconDeclaration::WadPicture { path, lump } => WeaponHudIcon::WadPicture {
            resource: ResourceRequest {
                content,
                path: path.clone(),
            },
            lump: lump.clone(),
        },
    }
}

/// Read a content-owned item icon (`readSourceItemIcon`).
pub fn read_source_item_icon(reader: &SaveReader, content: &ContentId) -> Result<WeaponHudIcon, ItemIconError> {
    let kind = reader.field("kind").choice_str(&["image", "wad-picture", "shader"])?;
    let resource = if kind == "shader" {
        reader.clone()
    } else {
        reader.field("resource")
    };
    if resource.field("content").string()? != content.as_str() {
        return Err(reader.fail("Item icon belongs to another content source").into());
    }
    if kind == "shader" {
        return Ok(WeaponHudIcon::Shader {
            content: content.clone(),
            name: normalize_resource_path(&reader.field("name").string()?)?,
        });
    }
    let path = normalize_resource_path(&resource.field("path").string()?)?;
    if kind == "image" {
        return Ok(WeaponHudIcon::Image {
            resource: ResourceRequest {
                content: content.clone(),
                path,
            },
        });
    }
    Ok(WeaponHudIcon::WadPicture {
        resource: ResourceRequest {
            content: content.clone(),
            path,
        },
        lump: read_lump(reader, &reader.field("lump").string()?)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{obj, str};

    fn content() -> ContentId {
        ContentId("q1:test:base:1".to_string())
    }

    #[test]
    fn reads_all_declaration_kinds() {
        let image = obj(vec![("kind", str("image")), ("path", str("icons/quad.png"))]);
        assert_eq!(
            read_item_icon_declaration(&SaveReader::new(&image)).unwrap(),
            SourceItemIconDeclaration::Image {
                path: "icons/quad.png".to_string()
            }
        );
        let wad = obj(vec![
            ("kind", str("wad-picture")),
            ("path", str("gfx.wad")),
            ("lump", str("QUAD")),
        ]);
        assert_eq!(
            read_item_icon_declaration(&SaveReader::new(&wad)).unwrap(),
            SourceItemIconDeclaration::WadPicture {
                path: "gfx.wad".to_string(),
                lump: "QUAD".to_string(),
            }
        );
        let shader = obj(vec![("kind", str("shader")), ("name", str("icons/quad"))]);
        assert_eq!(
            read_item_icon_declaration(&SaveReader::new(&shader)).unwrap(),
            SourceItemIconDeclaration::Shader {
                name: "icons/quad".to_string()
            }
        );
        // Lump names are 1-16 NUL-free units.
        let empty = obj(vec![
            ("kind", str("wad-picture")),
            ("path", str("gfx.wad")),
            ("lump", str("")),
        ]);
        let error = read_item_icon_declaration(&SaveReader::new(&empty)).unwrap_err();
        assert!(error.to_string().contains("valid WAD lump name"), "{error}");
        let long = obj(vec![
            ("kind", str("wad-picture")),
            ("path", str("gfx.wad")),
            ("lump", str("0123456789abcdefg")),
        ]);
        assert!(read_item_icon_declaration(&SaveReader::new(&long)).is_err());
    }

    #[test]
    fn resolves_icons_against_content() {
        let icon = SourceItemIconDeclaration::WadPicture {
            path: "gfx.wad".to_string(),
            lump: "QUAD".to_string(),
        };
        match resolve_item_icon(&icon, content()) {
            WeaponHudIcon::WadPicture { resource, lump } => {
                assert_eq!(resource.content, content());
                assert_eq!(resource.path, "gfx.wad");
                assert_eq!(lump, "QUAD");
            }
            other => panic!("expected WAD picture, got {other:?}"),
        }
        match resolve_item_icon(
            &SourceItemIconDeclaration::Shader {
                name: "icons/quad".to_string(),
            },
            content(),
        ) {
            WeaponHudIcon::Shader { content, name } => {
                assert_eq!(content, self::content());
                assert_eq!(name, "icons/quad");
            }
            other => panic!("expected shader, got {other:?}"),
        }
    }

    #[test]
    fn reads_owned_icons_and_rejects_foreign_content() {
        let owned = obj(vec![
            ("kind", str("image")),
            (
                "resource",
                obj(vec![
                    ("content", str("q1:test:base:1")),
                    ("path", str("icons/quad.png")),
                ]),
            ),
        ]);
        match read_source_item_icon(&SaveReader::new(&owned), &content()).unwrap() {
            WeaponHudIcon::Image { resource } => assert_eq!(resource.path, "icons/quad.png"),
            other => panic!("expected image, got {other:?}"),
        }
        let foreign = obj(vec![
            ("kind", str("image")),
            (
                "resource",
                obj(vec![
                    ("content", str("q2:test:base:1")),
                    ("path", str("icons/quad.png")),
                ]),
            ),
        ]);
        let error = read_source_item_icon(&SaveReader::new(&foreign), &content()).unwrap_err();
        assert!(error.to_string().contains("another content source"), "{error}");
        let shader = obj(vec![
            ("kind", str("shader")),
            ("content", str("q1:test:base:1")),
            ("name", str("icons/quad")),
        ]);
        assert!(matches!(
            read_source_item_icon(&SaveReader::new(&shader), &content()).unwrap(),
            WeaponHudIcon::Shader { .. }
        ));
    }
}
