//! Registered sound assets over mounted content.
//!
//! Donor provenance: `src/audio/bank.ts` (`SoundBank`). Content
//! access is injected through [`SoundContent`]; the donor's async
//! mount calls run synchronously here.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use super::error::AudioError;
use super::streams::{decode_sound_bytes, open_pcm_bytes, PcmStream};
use super::types::SoundAsset;
use super::wav::{decode_q3_wav, decode_quake_wav};
use crate::audio::SoundFamily;

/// Opened content file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedSound {
    /// Resource id.
    pub id: String,
    /// Mount content identity.
    pub content: String,
    /// File bytes.
    pub bytes: Vec<u8>,
}

/// Mounted content surface used by the bank.
pub trait SoundContent {
    /// Open a content path.
    fn open(&mut self, path: &str) -> Option<OpenedSound>;
}

/// Registered sounds with registration-scoped retention.
///
/// Assets are keyed by `(family, name)` with a borrowed lookup, so a cache
/// hit never touches content: no archive read, no digest, no `format!`.
/// The mount-reported resource id is indexed as an alias when it differs
/// from the normalized path (test mounts), keeping [`SoundBank::get`] exact.
pub struct SoundBank<Content: SoundContent> {
    content: Content,
    assets: HashMap<SoundFamily, HashMap<String, SoundAsset>>,
    touched: HashMap<SoundFamily, HashSet<String>>,
}

impl<Content: SoundContent> SoundBank<Content> {
    /// Bank over content.
    #[must_use]
    pub fn new(content: Content) -> Self {
        Self {
            content,
            assets: HashMap::new(),
            touched: HashMap::new(),
        }
    }

    /// Begin a registration pass.
    pub fn begin_registration(&mut self) {
        self.touched.clear();
    }

    /// Register a sound by name.
    ///
    /// The cache is checked before content is opened, so a repeat play
    /// skips the archive read, the content digest, and every `format!`.
    pub fn register(&mut self, name: &str, family: SoundFamily) -> Result<Option<SoundAsset>, AudioError> {
        let path = normalize_name(name);
        if let Some(prior) = self.assets.get(&family).and_then(|by_name| by_name.get(path.as_ref())) {
            Self::touch(&mut self.touched, family, path.as_ref());
            Self::touch(&mut self.touched, family, &prior.resource);
            return Ok(Some(prior.clone()));
        }
        let Some(opened) = self.content.open(path.as_ref()) else {
            return Ok(None);
        };
        // Same bytes already decoded under another name: alias the path to
        // the existing asset instead of decoding again.
        let aliased: Option<SoundAsset> = self
            .assets
            .get(&family)
            .and_then(|by_name| by_name.get(opened.id.as_str()))
            .cloned();
        if let Some(asset) = aliased {
            Self::touch(&mut self.touched, family, path.as_ref());
            Self::touch(&mut self.touched, family, asset.resource.as_str());
            self.assets
                .get_mut(&family)
                .expect("aliased family is registered")
                .insert(path.into_owned(), asset.clone());
            return Ok(Some(asset));
        }
        let owned = path.into_owned();
        let pcm = if opened.bytes.first() == Some(&82) {
            if family == SoundFamily::Q3 {
                decode_q3_wav(&opened.bytes, &owned)?.pcm
            } else {
                decode_quake_wav(&opened.bytes, &owned)?.pcm
            }
        } else {
            decode_sound_bytes(&opened.bytes, &owned)?
        };
        let asset = SoundAsset {
            resource: opened.id,
            name: owned,
            pcm: Rc::new(pcm),
        };
        Self::touch(&mut self.touched, family, asset.name.as_str());
        Self::touch(&mut self.touched, family, asset.resource.as_str());
        let by_name = self.assets.entry(family).or_default();
        by_name.insert(asset.name.clone(), asset.clone());
        if asset.resource != asset.name {
            by_name.insert(asset.resource.clone(), asset.clone());
        }
        Ok(Some(asset))
    }

    /// Mark a key touched, allocating only on the first touch per pass.
    fn touch(touched: &mut HashMap<SoundFamily, HashSet<String>>, family: SoundFamily, key: &str) {
        let set = touched.entry(family).or_default();
        if !set.contains(key) {
            set.insert(key.to_string());
        }
    }

    /// Register a Q2 sexed/player sound.
    pub fn register_sexed_sound(&mut self, base: &str, model: &str) -> Result<Option<SoundAsset>, AudioError> {
        if !base.starts_with('*') {
            return self.register(base, SoundFamily::Q2);
        }
        let selected = model
            .split('/')
            .next()
            .filter(|part| !part.is_empty())
            .unwrap_or("male");
        let name = &base[1..];
        if let Some(asset) = self.register(&format!("#players/{selected}/{name}"), SoundFamily::Q2)? {
            return Ok(Some(asset));
        }
        self.register(&format!("player/male/{name}"), SoundFamily::Q2)
    }

    /// Look up a registered asset.
    #[must_use]
    pub fn get(&self, resource: &str, family: SoundFamily) -> Option<&SoundAsset> {
        self.assets.get(&family)?.get(resource)
    }

    /// Drop assets untouched since [`SoundBank::begin_registration`].
    pub fn end_registration(&mut self) {
        for (family, by_name) in self.assets.iter_mut() {
            match self.touched.get(family) {
                Some(keep) => by_name.retain(|key, _| keep.contains(key)),
                None => by_name.clear(),
            }
        }
    }

    /// Open a music stream, optionally pinned to a mount.
    pub fn open_music(&mut self, path: &str, source: Option<&str>) -> Result<Option<Box<dyn PcmStream>>, AudioError> {
        let Some(opened) = self.content.open(path) else {
            return Ok(None);
        };
        if source.is_some_and(|source| opened.content != source) {
            return Ok(None);
        }
        Ok(Some(open_pcm_bytes(&opened.bytes, path)?))
    }

    /// Drop every asset.
    pub fn clear(&mut self) {
        self.assets.clear();
        self.touched.clear();
    }
}

/// Normalize a registry name to its content path without allocating when the
/// name already carries its prefix.
fn normalize_name(name: &str) -> Cow<'_, str> {
    if let Some(stripped) = name.strip_prefix('#') {
        Cow::Borrowed(stripped)
    } else if name.starts_with("sound/") {
        Cow::Borrowed(name)
    } else {
        Cow::Owned(format!("sound/{name}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeContent {
        files: HashMap<String, OpenedSound>,
        opens: usize,
    }

    impl SoundContent for FakeContent {
        fn open(&mut self, path: &str) -> Option<OpenedSound> {
            self.opens += 1;
            self.files.get(path).cloned()
        }
    }

    fn wav() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&40u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&11025u32.to_le_bytes());
        bytes.extend_from_slice(&11025u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&[0, 128, 255, 64]);
        bytes
    }

    fn bank() -> SoundBank<FakeContent> {
        let mut files = HashMap::new();
        files.insert(
            "sound/shot.wav".to_string(),
            OpenedSound {
                id: "shot".to_string(),
                content: "base".to_string(),
                bytes: wav(),
            },
        );
        files.insert(
            "sound/alias.wav".to_string(),
            OpenedSound {
                id: "shot".to_string(),
                content: "base".to_string(),
                bytes: wav(),
            },
        );
        SoundBank::new(FakeContent { files, opens: 0 })
    }

    fn opens(bank: &SoundBank<FakeContent>) -> usize {
        bank.content.opens
    }

    #[test]
    fn registers_and_retains() {
        let mut bank = bank();
        bank.begin_registration();
        let asset = bank.register("shot.wav", SoundFamily::Q3).unwrap().unwrap();
        assert_eq!(asset.name, "sound/shot.wav");
        assert_eq!(asset.pcm.samples.len(), 4);
        assert!(bank.register("missing.wav", SoundFamily::Q3).unwrap().is_none());
        assert!(bank.get("shot", SoundFamily::Q3).is_some());
        assert!(bank.get("shot", SoundFamily::Q1).is_none());
        bank.begin_registration();
        bank.end_registration();
        assert!(bank.get("shot", SoundFamily::Q3).is_none());
        bank.clear();
    }

    #[test]
    fn cached_register_skips_content_open() {
        let mut bank = bank();
        bank.begin_registration();
        bank.register("shot.wav", SoundFamily::Q3).unwrap().unwrap();
        assert_eq!(opens(&bank), 1);
        // Repeat plays hit the cache: no archive read, no digest.
        bank.register("shot.wav", SoundFamily::Q3).unwrap().unwrap();
        bank.register("sound/shot.wav", SoundFamily::Q3).unwrap().unwrap();
        bank.register("#sound/shot.wav", SoundFamily::Q3).unwrap().unwrap();
        assert_eq!(opens(&bank), 1);
        // Other families decode separately.
        bank.register("shot.wav", SoundFamily::Q1).unwrap().unwrap();
        assert_eq!(opens(&bank), 2);
        bank.register("shot.wav", SoundFamily::Q1).unwrap().unwrap();
        assert_eq!(opens(&bank), 2);
    }

    #[test]
    fn alias_paths_share_one_decode() {
        let mut bank = bank();
        bank.begin_registration();
        let first = bank.register("shot.wav", SoundFamily::Q2).unwrap().unwrap();
        let second = bank.register("alias.wav", SoundFamily::Q2).unwrap().unwrap();
        assert!(Rc::ptr_eq(&first.pcm, &second.pcm));
        assert_eq!(opens(&bank), 2);
        // Both spellings are cached now.
        bank.register("alias.wav", SoundFamily::Q2).unwrap().unwrap();
        bank.register("shot.wav", SoundFamily::Q2).unwrap().unwrap();
        assert_eq!(opens(&bank), 2);
        // Retention is per spelling: the untouched alias drops while the id
        // survives, and re-registering the alias re-opens once but reuses
        // the surviving decode.
        bank.begin_registration();
        bank.register("shot.wav", SoundFamily::Q2).unwrap().unwrap();
        bank.end_registration();
        assert!(bank.get("shot", SoundFamily::Q2).is_some());
        assert!(bank.get("sound/alias.wav", SoundFamily::Q2).is_none());
        let revived = bank.register("alias.wav", SoundFamily::Q2).unwrap().unwrap();
        assert_eq!(opens(&bank), 3);
        assert!(Rc::ptr_eq(&first.pcm, &revived.pcm));
    }
}
