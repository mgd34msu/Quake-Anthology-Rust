//! Image format policy from texture console controls.
//!
//! Donor provenance: `src/render/scene/image-policy.ts`
//! (`parseImageFormats`, `snapshotImagePolicy`, `imagePolicyFromControls,
//! `DEFAULT_IMAGE_POLICY`) plus a minimal port of the `parseQ2Token`
//! tokenizer from `src/core/common-parse.ts` (whitespace/`//`-comment
//! skipping, quoted strings, plain words).

/// Texture usage class for override selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageUsage {
    /// Model skins.
    Skin,
    /// Sprites.
    Sprite,
    /// World walls.
    Wall,
    /// 2D pictures.
    Picture,
    /// Sky textures.
    Sky,
}

/// Loadable image container format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    /// PNG.
    Png,
    /// JPEG (short name).
    Jpg,
    /// Targa.
    Tga,
    /// JPEG (long name).
    Jpeg,
    /// Bitmap.
    Bmp,
    /// GIF.
    Gif,
}

/// Resolved texture override policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImagePolicy {
    /// Override detail level.
    pub override_level: i32,
    /// Usages the override applies to.
    pub override_usages: Vec<ImageUsage>,
    /// Accepted formats, or [`None`] for source fidelity.
    pub formats: Option<Vec<ImageFormat>>,
}

/// Raw console controls backing [`image_policy_from_controls`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageControls {
    /// Override detail level.
    pub override_level: i32,
    /// Usage bitmask: skin=1 sprite=2 wall=4 picture=8 sky=16.
    pub override_mask: i32,
    /// Format list, or "source" for source fidelity.
    pub formats: String,
}

const FORMATS: [ImageFormat; 6] = [
    ImageFormat::Png,
    ImageFormat::Jpg,
    ImageFormat::Tga,
    ImageFormat::Jpeg,
    ImageFormat::Bmp,
    ImageFormat::Gif,
];

const FORMAT_NAMES: [&str; 6] = ["png", "jpg", "tga", "jpeg", "bmp", "gif"];

const USAGE_BITS: [(ImageUsage, i32); 5] = [
    (ImageUsage::Skin, 1),
    (ImageUsage::Sprite, 2),
    (ImageUsage::Wall, 4),
    (ImageUsage::Picture, 8),
    (ImageUsage::Sky, 16),
];

/// Byte at an index, or NUL past the end.
fn char_at(data: &[u8], index: usize) -> u8 {
    data.get(index).copied().unwrap_or(0)
}

/// Next Q2 token, advancing past whitespace, `//` comments, and the word.
fn next_q2_token(data: &[u8], index: &mut usize) -> String {
    let mut at = *index;
    loop {
        let mut current = char_at(data, at);
        while current <= 32 {
            if current == 0 {
                *index = at;
                return String::new();
            }
            at += 1;
            current = char_at(data, at);
        }
        if current == b'/' && char_at(data, at + 1) == b'/' {
            while char_at(data, at) != 0 && char_at(data, at) != b'\n' {
                at += 1;
            }
            continue;
        }
        break;
    }

    if char_at(data, at) == b'"' {
        at += 1;
        let mut token = Vec::new();
        loop {
            let current = char_at(data, at);
            at += 1;
            if current == b'"' || current == 0 {
                *index = at;
                return String::from_utf8_lossy(&token).into_owned();
            }
            token.push(current);
        }
    }

    let mut token = Vec::new();
    loop {
        token.push(char_at(data, at));
        at += 1;
        if char_at(data, at) <= 32 {
            break;
        }
    }
    *index = at;
    String::from_utf8_lossy(&token).into_owned()
}

/// Parse a format list: named tokens first, then legacy initials.
#[must_use]
pub fn parse_image_formats(value: &str) -> Vec<ImageFormat> {
    let mut result: Vec<ImageFormat> = Vec::new();
    let data = value.as_bytes();
    let mut index = 0usize;
    while index < data.len() {
        let start = index;
        let word = next_q2_token(data, &mut index).to_lowercase();
        if index == start {
            break;
        }
        if let Some(position) = FORMAT_NAMES.iter().position(|name| *name == word) {
            let format = FORMATS[position];
            if !result.contains(&format) {
                result.push(format);
            }
            continue;
        }
        for letter in word.chars() {
            let initial = FORMAT_NAMES
                .iter()
                .position(|name| name.starts_with(letter))
                .map(|position| FORMATS[position]);
            if let Some(format) = initial {
                if !result.contains(&format) {
                    result.push(format);
                }
            }
        }
    }
    result
}

/// Detached copy of a policy.
#[must_use]
pub fn snapshot_image_policy(policy: &ImagePolicy) -> ImagePolicy {
    policy.clone()
}

/// Resolve console controls into a policy.
#[must_use]
pub fn image_policy_from_controls(controls: &ImageControls) -> ImagePolicy {
    let usages = USAGE_BITS
        .iter()
        .filter(|(_, bit)| controls.override_mask & *bit != 0)
        .map(|(usage, _)| *usage)
        .collect();
    let formats = if controls.formats.trim().to_lowercase() == "source" {
        None
    } else {
        Some(parse_image_formats(&controls.formats))
    };
    snapshot_image_policy(&ImagePolicy {
        override_level: controls.override_level,
        override_usages: usages,
        formats,
    })
}

/// Default policy: level 1, every usage, all six formats.
#[must_use]
pub fn default_image_policy() -> ImagePolicy {
    image_policy_from_controls(&ImageControls {
        override_level: 1,
        override_mask: -1,
        formats: "png jpg tga jpeg bmp gif".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_tokens_parse_in_order() {
        assert_eq!(parse_image_formats("tga png"), vec![ImageFormat::Tga, ImageFormat::Png]);
        assert_eq!(parse_image_formats("JPEG"), vec![ImageFormat::Jpeg]);
    }

    #[test]
    fn legacy_initials_expand_with_first_match_wins() {
        assert_eq!(parse_image_formats("pt"), vec![ImageFormat::Png, ImageFormat::Tga]);
        assert_eq!(parse_image_formats("j"), vec![ImageFormat::Jpg]);
        assert_eq!(parse_image_formats("z q"), Vec::<ImageFormat>::new());
    }

    #[test]
    fn tokens_deduplicate_and_skip_comments_and_quotes() {
        assert_eq!(parse_image_formats("png png p"), vec![ImageFormat::Png]);
        assert_eq!(
            parse_image_formats("png // jpg\n\"tga\""),
            vec![ImageFormat::Png, ImageFormat::Tga]
        );
        assert_eq!(parse_image_formats("   "), Vec::<ImageFormat>::new());
    }

    #[test]
    fn source_disables_format_filtering() {
        let policy = image_policy_from_controls(&ImageControls {
            override_level: 0,
            override_mask: 0,
            formats: "  Source ".to_string(),
        });
        assert_eq!(policy.formats, None);
        let filtered = image_policy_from_controls(&ImageControls {
            override_level: 0,
            override_mask: 0,
            formats: "png".to_string(),
        });
        assert_eq!(filtered.formats, Some(vec![ImageFormat::Png]));
    }

    #[test]
    fn mask_bits_map_to_usages() {
        let policy = image_policy_from_controls(&ImageControls {
            override_level: 2,
            override_mask: 1 | 4,
            formats: "png".to_string(),
        });
        assert_eq!(policy.override_level, 2);
        assert_eq!(policy.override_usages, vec![ImageUsage::Skin, ImageUsage::Wall]);
        let none = image_policy_from_controls(&ImageControls {
            override_level: 0,
            override_mask: 0,
            formats: "png".to_string(),
        });
        assert!(none.override_usages.is_empty());
    }

    #[test]
    fn defaults_cover_everything() {
        let policy = default_image_policy();
        assert_eq!(policy.override_level, 1);
        assert_eq!(
            policy.override_usages,
            vec![
                ImageUsage::Skin,
                ImageUsage::Sprite,
                ImageUsage::Wall,
                ImageUsage::Picture,
                ImageUsage::Sky,
            ]
        );
        assert_eq!(
            policy.formats,
            Some(vec![
                ImageFormat::Png,
                ImageFormat::Jpg,
                ImageFormat::Tga,
                ImageFormat::Jpeg,
                ImageFormat::Bmp,
                ImageFormat::Gif,
            ])
        );
    }

    #[test]
    fn snapshots_detach() {
        let policy = default_image_policy();
        let snapshot = snapshot_image_policy(&policy);
        assert_eq!(snapshot, policy);
    }
}
