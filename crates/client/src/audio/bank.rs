//! Registered sound assets over mounted content.
//!
//! Donor provenance: `src/audio/bank.ts` (`SoundBank`). Content
//! access is injected through [`SoundContent`]; the donor's async
//! mount calls run synchronously here.

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
pub struct SoundBank<Content: SoundContent> {
    content: Content,
    assets: HashMap<String, SoundAsset>,
    touched: HashSet<String>,
}

impl<Content: SoundContent> SoundBank<Content> {
    /// Bank over content.
    #[must_use]
    pub fn new(content: Content) -> Self {
        Self {
            content,
            assets: HashMap::new(),
            touched: HashSet::new(),
        }
    }

    /// Begin a registration pass.
    pub fn begin_registration(&mut self) {
        self.touched.clear();
    }

    /// Register a sound by name.
    pub fn register(&mut self, name: &str, family: SoundFamily) -> Result<Option<SoundAsset>, AudioError> {
        let path = if let Some(stripped) = name.strip_prefix('#') {
            stripped.to_string()
        } else if name.starts_with("sound/") {
            name.to_string()
        } else {
            format!("sound/{name}")
        };
        let Some(opened) = self.content.open(&path) else {
            return Ok(None);
        };
        let key = format!("{}:{}", family_name(family), opened.id);
        self.touched.insert(key.clone());
        if let Some(prior) = self.assets.get(&key) {
            return Ok(Some(prior.clone()));
        }
        let pcm = if opened.bytes.first() == Some(&82) {
            if family == SoundFamily::Q3 {
                decode_q3_wav(&opened.bytes, &path)?.pcm
            } else {
                decode_quake_wav(&opened.bytes, &path)?.pcm
            }
        } else {
            decode_sound_bytes(&opened.bytes, &path)?
        };
        let asset = SoundAsset {
            resource: opened.id,
            name: path,
            pcm: Rc::new(pcm),
        };
        self.assets.insert(key, asset.clone());
        Ok(Some(asset))
    }

    /// Register a Q2 sexed/player sound.
    pub fn register_sexed_sound(&mut self, base: &str, model: &str) -> Result<Option<SoundAsset>, AudioError> {
        if !base.starts_with('*') {
            return self.register(base, SoundFamily::Q2);
        }
        let selected = model.split('/').next().filter(|part| !part.is_empty()).unwrap_or("male");
        let name = &base[1..];
        if let Some(asset) = self.register(&format!("#players/{selected}/{name}"), SoundFamily::Q2)? {
            return Ok(Some(asset));
        }
        self.register(&format!("player/male/{name}"), SoundFamily::Q2)
    }

    /// Look up a registered asset.
    #[must_use]
    pub fn get(&self, resource: &str, family: SoundFamily) -> Option<&SoundAsset> {
        self.assets.get(&format!("{}:{resource}", family_name(family)))
    }

    /// Drop assets untouched since [`SoundBank::begin_registration`].
    pub fn end_registration(&mut self) {
        self.assets.retain(|key, _| self.touched.contains(key));
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

fn family_name(family: SoundFamily) -> &'static str {
    match family {
        SoundFamily::Q1 => "q1",
        SoundFamily::Q2 => "q2",
        SoundFamily::Q3 => "q3",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeContent {
        files: HashMap<String, OpenedSound>,
    }

    impl SoundContent for FakeContent {
        fn open(&mut self, path: &str) -> Option<OpenedSound> {
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
        SoundBank::new(FakeContent { files })
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
}
