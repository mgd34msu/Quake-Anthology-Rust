use super::{ImageRole, ImageSettings, ImageUse, ResourceError};
use qa_content::vfs::Vfs;
use qa_core::primitives::RuleSetId;
use qa_formats::{FormatError, image::ImageFormat, text::Tokens};

/// A cold parsed order. A JPEG decoder may have more than one file suffix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageSearchOrder {
    suffixes: [Option<&'static str>; 6],
    source: bool,
}
impl Default for ImageSearchOrder {
    fn default() -> Self {
        Self {
            suffixes: [Some("png"), Some("jpg"), Some("tga"), None, None, None],
            source: false,
        }
    }
}
impl ImageSearchOrder {
    pub fn parse(text: &[u8]) -> Result<Self, FormatError> {
        if text.trim_ascii().eq_ignore_ascii_case(b"source") {
            return Ok(Self {
                suffixes: [None; 6],
                source: true,
            });
        }
        let mut order = Self {
            suffixes: [None; 6],
            source: false,
        };
        let mut tokens = Tokens::entity(text, b"", true, usize::MAX);
        while let Some(token) = tokens.next()? {
            if let Some((suffix, _)) =
                replacements().find(|(suffix, _)| token.eq_ignore_ascii_case(suffix.as_bytes()))
            {
                order.add(suffix);
            } else {
                for byte in token {
                    if let Some((suffix, _)) = replacements()
                        .find(|(suffix, _)| suffix.as_bytes()[0].eq_ignore_ascii_case(byte))
                    {
                        order.add(suffix);
                    }
                }
            }
        }
        Ok(order)
    }
    fn add(&mut self, suffix: &'static str) {
        if !self.suffixes.contains(&Some(suffix))
            && let Some(slot) = self.suffixes.iter_mut().find(|slot| slot.is_none())
        {
            *slot = Some(suffix);
        }
    }
    fn fallback_suffixes(self, source: RuleSetId, role: ImageRole) -> [Option<&'static str>; 8] {
        let native = if matches!(source, RuleSetId::Quake | RuleSetId::QuakeWorld) {
            [Some("lmp"), Some("pcx")]
        } else if matches!(source, RuleSetId::Quake2 | RuleSetId::Quake2Rerelease)
            && role == ImageRole::Surface
        {
            [Some("wal"), None]
        } else {
            [Some("pcx"), None]
        };
        if self.source {
            match source {
                RuleSetId::Quake | RuleSetId::QuakeWorld => [
                    Some("lmp"),
                    Some("tga"),
                    Some("jpg"),
                    Some("png"),
                    Some("jpeg"),
                    Some("pcx"),
                    Some("bmp"),
                    Some("gif"),
                ],
                RuleSetId::Quake2 | RuleSetId::Quake2Rerelease => [
                    Some("png"),
                    Some("jpg"),
                    Some("tga"),
                    Some("jpeg"),
                    Some("bmp"),
                    Some("gif"),
                    native[0],
                    None,
                ],
                RuleSetId::Quake3 => [
                    Some("tga"),
                    Some("jpg"),
                    Some("png"),
                    Some("jpeg"),
                    Some("pcx"),
                    Some("bmp"),
                    Some("gif"),
                    None,
                ],
            }
        } else {
            let mut suffixes = [None; 8];
            suffixes[..6].copy_from_slice(&self.suffixes);
            suffixes[6..].copy_from_slice(&native);
            suffixes
        }
    }
    fn color_suffixes(self, source: RuleSetId) -> impl Iterator<Item = &'static str> {
        self.fallback_suffixes(source, ImageRole::Pic)
            .into_iter()
            .flatten()
            .filter(|suffix| !matches!(*suffix, "pcx" | "lmp" | "wal"))
    }
    pub(super) fn select(
        self,
        vfs: &Vfs,
        name: &str,
        source: RuleSetId,
        settings: ImageSettings,
        usage: ImageUse,
        rgba_override: Option<&str>,
        embedded: bool,
    ) -> Result<Option<SelectedFile>, ResourceError> {
        let base = stem(name);
        let explicit = base.len() != name.len();
        let original = SourceFormat::from_path(name);
        let native = embedded
            || matches!(
                original,
                Some(SourceFormat::Wal | SourceFormat::Raster(ImageFormat::Pcx | ImageFormat::Lmp))
            );
        let enabled = settings.override_textures >= 1
            && usage
                .role
                .override_bit()
                .is_some_and(|bit| settings.texture_overrides & bit != 0);
        let replace = enabled && (native || settings.override_textures > 1 && original.is_some());
        let rr_forced = source == RuleSetId::Quake2Rerelease && replace;
        let mut paths: [Option<String>; 12] = std::array::from_fn(|_| None);
        let mut count = 0;
        let mut try_path = |path: String| -> Result<Option<SelectedFile>, ResourceError> {
            if paths[..count].iter().flatten().any(|old| *old == path) {
                return Ok(None);
            }
            if vfs.open(path.as_bytes()).is_some() {
                let format = SourceFormat::from_path(&path)
                    .ok_or(ResourceError::Asset("unknown image format"))?;
                return Ok(Some(SelectedFile { path, format }));
            }
            if count < paths.len() {
                paths[count] = Some(path);
                count += 1;
            }
            Ok(None)
        };
        if let Some(path) = rgba_override
            && let Some(selected) = try_path(path.to_owned())?
        {
            return Ok(Some(selected));
        }
        if replace {
            for suffix in self.color_suffixes(source) {
                if let Some(selected) = try_path(format!("{base}.{suffix}"))? {
                    return Ok(Some(selected));
                }
            }
        }
        if embedded {
            return Ok(None);
        }
        if !rr_forced {
            if explicit && (original.is_some() || source != RuleSetId::Quake2Rerelease) {
                if let Some(selected) = try_path(name.to_owned())? {
                    return Ok(Some(selected));
                }
            } else if matches!(source, RuleSetId::Quake2 | RuleSetId::Quake2Rerelease)
                && usage.role == ImageRole::Surface
                && let Some(selected) = try_path(format!("{base}.wal"))?
            {
                return Ok(Some(selected));
            }
        }
        for suffix in self
            .fallback_suffixes(source, usage.role)
            .into_iter()
            .flatten()
        {
            if let Some(selected) = try_path(format!("{base}.{suffix}"))? {
                return Ok(Some(selected));
            }
        }
        Ok(None)
    }
}
fn replacements() -> impl Iterator<Item = (&'static str, ImageFormat)> {
    ImageFormat::SUFFIXES
        .iter()
        .copied()
        .filter(|(_, format)| !matches!(format, ImageFormat::Pcx | ImageFormat::Lmp))
}
impl ImageRole {
    pub(super) fn override_bit(self) -> Option<u32> {
        Some(match self {
            Self::Pic => 1,
            Self::Font => 2,
            Self::Skin => 4,
            Self::Sprite => 8,
            Self::Surface => 16,
            Self::LayeredSky | Self::CubeSky => 32,
            Self::NativeDefault | Self::Lightmap => return None,
        })
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SourceFormat {
    Raster(ImageFormat),
    Wal,
}
impl SourceFormat {
    pub(super) fn retains_indices(self, source: RuleSetId) -> bool {
        matches!(source, RuleSetId::Quake2 | RuleSetId::Quake2Rerelease)
            && matches!(self, Self::Wal | Self::Raster(ImageFormat::Pcx))
            || matches!(source, RuleSetId::Quake | RuleSetId::QuakeWorld)
                && self == Self::Raster(ImageFormat::Lmp)
    }
    pub(super) fn native_fringe(self, presentation: RuleSetId) -> bool {
        self == Self::Raster(ImageFormat::Pcx)
            || self == Self::Wal
                && matches!(presentation, RuleSetId::Quake2 | RuleSetId::Quake2Rerelease)
    }
    pub(super) fn from_path(path: &str) -> Option<Self> {
        let component = path.rsplit(['/', '\\']).next()?;
        let (_, suffix) = component.rsplit_once('.')?;
        if suffix.eq_ignore_ascii_case("wal") {
            Some(Self::Wal)
        } else {
            ImageFormat::from_path(component.as_bytes()).map(Self::Raster)
        }
    }
}
pub(super) fn stem(path: &str) -> &str {
    let component = path.rsplit(['/', '\\']).next().unwrap_or(path);
    component
        .rsplit_once('.')
        .map_or(path, |(_, suffix)| &path[..path.len() - suffix.len() - 1])
}
pub(super) struct SelectedFile {
    pub path: String,
    pub format: SourceFormat,
}
