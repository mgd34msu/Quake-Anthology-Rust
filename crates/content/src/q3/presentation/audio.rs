//! Quake III presentation: audio.
//!
//! Donor provenance: `src/content/q3/presentation/audio.ts`.

use qa_core::math::Vec3;
use std::collections::HashMap;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::mirrors_present_hud::*;

/// Sound registration row (`SoundRegistration`).
#[derive(Debug, Clone, PartialEq)]
pub struct SoundRegistration {
    /// Path.
    pub path: String,
    /// Compressed intent.
    pub compressed: bool,
    /// Asset.
    pub sound: Option<SoundAsset>,
}

/// Cgame presentation sound bank (`Q3PresentationSoundBank`).
#[allow(clippy::type_complexity)]
pub struct Q3PresentationSoundBank {
    /// Engine bank.
    pub bank: Shared<dyn SoundBank>,
    /// Zero sound.
    pub zero_sound: Option<SoundAsset>,
    /// Synchronous loader.
    pub load_sync: Box<dyn Fn(&str, bool) -> Option<SoundAsset>>,
    /// Registered assets.
    registered: Vec<SoundAsset>,
    /// In-flight registrations.
    operations: u32,
    /// Requests by path.
    requests: HashMap<String, SoundRegistration>,
}

impl Q3PresentationSoundBank {
    /// Assemble a bank.
    #[allow(clippy::type_complexity)]
    pub fn new(
        bank: Shared<dyn SoundBank>,
        zero_sound: Option<SoundAsset>,
        load_sync: Box<dyn Fn(&str, bool) -> Option<SoundAsset>>,
    ) -> Self {
        Self {
            bank,
            zero_sound,
            load_sync,
            registered: Vec::new(),
            operations: 0,
            requests: HashMap::new(),
        }
    }

    /// Whether a PCM is the zero sound.
    fn is_zero(&self, sound: &PcmSound) -> bool {
        self.zero_sound
            .as_ref()
            .is_some_and(|zero| Rc::ptr_eq(&zero.pcm, sound))
    }

    /// Map an asset to nullable PCM, hiding the zero sound.
    fn exposed(&self, sound: &Option<SoundAsset>) -> Option<PcmSound> {
        match sound {
            None => None,
            Some(asset) if self.is_zero(&asset.pcm) => None,
            Some(asset) => Some(asset.pcm.clone()),
        }
    }

    /// Track a fresh asset.
    fn track(&mut self, sound: &Option<SoundAsset>) {
        if let Some(asset) = sound {
            if !self.is_zero(&asset.pcm) && !self.registered.iter().any(|entry| Rc::ptr_eq(&entry.pcm, &asset.pcm)) {
                self.registered.push(asset.clone());
            }
        }
    }

    /// Capture a checkpoint (`captureCheckpoint`).
    pub fn capture_checkpoint(&self) -> Vec<SoundCheckpointRow> {
        if self.operations != 0 {
            panic!("Cannot checkpoint pending sound registration");
        }
        self.requests
            .values()
            .map(|row| SoundCheckpointRow {
                path: row.path.clone(),
                compressed: row.compressed,
                handle: self.index_for_sound(&self.exposed(&row.sound)),
                resource: row.sound.as_ref().and_then(|asset| asset.resource.clone()),
            })
            .collect()
    }

    /// Restore a checkpoint (`restoreCheckpoint`).
    pub fn restore_checkpoint(&mut self, rows: &[SoundCheckpointRow]) {
        if self.operations != 0 || !self.requests.is_empty() {
            panic!("Sound restore requires an empty owner");
        }
        for row in rows {
            let sound = self.register_sound(Some(&row.path), row.compressed);
            let entry = self.requests.get(&row.path);
            if self.index_for_sound(&sound) != row.handle
                || entry
                    .and_then(|entry| entry.sound.as_ref())
                    .and_then(|asset| asset.resource.clone())
                    != row.resource
            {
                panic!("sound resource binding changed");
            }
        }
    }
}

impl ClientSoundBank for Q3PresentationSoundBank {
    fn register_sound(&mut self, path: Option<&str>, compressed: bool) -> Option<PcmSound> {
        let Some(path) = path else {
            panic!("S_RegisterSound dereferences NULL name at strlen");
        };
        if path.is_empty() || path.starts_with('*') {
            return None;
        }
        if let Some(prior) = self.requests.get(path) {
            return self.exposed(&prior.sound.clone());
        }
        self.operations += 1;
        let sound = self.bank.borrow_mut().register(path, "q3");
        self.operations -= 1;
        self.requests.insert(
            path.to_string(),
            SoundRegistration {
                path: path.to_string(),
                compressed,
                sound: sound.clone(),
            },
        );
        self.track(&sound);
        self.exposed(&sound)
    }

    fn sound(&mut self, path: Option<&str>, compressed: bool) -> Option<PcmSound> {
        let path = path?;
        if !self.requests.contains_key(path) {
            let sound = (self.load_sync)(path, compressed);
            self.track(&sound);
            self.requests.insert(
                path.to_string(),
                SoundRegistration {
                    path: path.to_string(),
                    compressed,
                    sound,
                },
            );
        }
        let entry = self.requests.get(path).cloned().unwrap_or(SoundRegistration {
            path: path.to_string(),
            compressed,
            sound: None,
        });
        self.exposed(&entry.sound)
    }

    fn index_for_sound(&self, sound: &Option<PcmSound>) -> i32 {
        match sound {
            None => 0,
            Some(pcm) if self.is_zero(pcm) => 0,
            Some(pcm) => self
                .registered
                .iter()
                .position(|entry| Rc::ptr_eq(&entry.pcm, pcm))
                .map(|index| index as i32 + 1)
                .unwrap_or_else(|| panic!("PCM does not belong to this cgame sound bank")),
        }
    }

    fn asset(&self, sound: &Option<PcmSound>) -> Option<SoundAsset> {
        let index = self.index_for_sound(sound);
        if index == 0 {
            return self.zero_sound.clone();
        }
        self.registered
            .get(index as usize - 1)
            .cloned()
            .or_else(|| self.zero_sound.clone())
    }

    fn sound_at_index(&self, index: i32) -> Option<PcmSound> {
        if index == 0 {
            return None;
        }
        match self.registered.get(index as usize - 1) {
            Some(sound) if index > 0 => Some(sound.pcm.clone()),
            _ => panic!("Q3 sound handle {index} is not registered"),
        }
    }

    fn sound_for_index(&self, index: i32) -> Option<PcmSound> {
        if index == 0 {
            return None;
        }
        if index < 0 {
            return None;
        }
        self.registered.get(index as usize - 1).map(|asset| asset.pcm.clone())
    }

    fn registrations(&self) -> Vec<SoundRegistration> {
        self.requests.values().cloned().collect()
    }
}

/// Cgame audio target (`Q3AudioTarget`).
pub trait Q3AudioTarget {
    /// Viewing seat.
    fn seat(&self) -> SeatId;
    /// Sound bank.
    fn sounds(&self) -> Shared<dyn ClientSoundBank>;
    /// Actor for a source number.
    fn actor(&self, source: i32) -> ActorId;
    /// Frame number.
    fn frame_number(&self) -> i32;
    /// Play a one-shot.
    fn play(&mut self, sound: PlaySound);
    /// Play a loop.
    fn loop_sound(&mut self, sound: LoopSound);
    /// Update an actor position.
    fn update_actor(&mut self, actor: ActorId, position: Vec3);
    /// Stop an audience loop.
    fn stop_loop(&mut self, seat: SeatId, actor: ActorId);
}

/// Cgame one-shot and loop calls (`Q3PresentationAudio`).
pub struct Q3PresentationAudio {
    /// Target.
    pub target: Shared<dyn Q3AudioTarget>,
}

impl Q3PresentationAudio {
    /// Wrap a target.
    pub fn new(target: Shared<dyn Q3AudioTarget>) -> Self {
        Self { target }
    }

    /// Start a source sound (`startSourceSound`).
    pub fn start_source_sound(&self, sound: Option<PcmSound>, options: &StartSoundOptions) {
        let asset = self.target.borrow().sounds().borrow().asset(&sound);
        let Some(asset) = asset else {
            return;
        };
        let actor = if options.entity < 0 {
            None
        } else {
            Some(self.target.borrow().actor(options.entity))
        };
        let origin = match options.origin {
            StartSoundOrigin::Entity { entity } => SoundOrigin::Actor {
                actor: self.target.borrow().actor(entity),
            },
            StartSoundOrigin::Fixed { position } => SoundOrigin::Fixed { position },
            StartSoundOrigin::Local => SoundOrigin::Local,
        };
        let seat = self.target.borrow().seat();
        let attenuation = if matches!(origin, SoundOrigin::Local) { 0.0 } else { 1.0 };
        self.target.borrow_mut().play(PlaySound {
            sound: asset,
            actor,
            origin,
            seat,
            channel: options.channel,
            volume: options.volume / 127.0,
            attenuation,
        });
    }

    /// Start a placed sound (`startSound`).
    pub fn start_sound(&self, origin: Option<Vec3>, entity: i32, channel: i32, sound: Option<PcmSound>) {
        let asset = self.target.borrow().sounds().borrow().asset(&sound);
        let Some(asset) = asset else {
            return;
        };
        let actor = if entity < 0 {
            None
        } else {
            Some(self.target.borrow().actor(entity))
        };
        let source = match origin {
            Some(position) => SoundOrigin::Fixed { position },
            None => match actor {
                None => panic!("Entity-attached sound requires a source actor"),
                Some(actor) => SoundOrigin::Actor { actor },
            },
        };
        let seat = self.target.borrow().seat();
        self.target.borrow_mut().play(PlaySound {
            sound: asset,
            actor,
            origin: source,
            seat,
            channel,
            volume: 1.0,
            attenuation: 1.0,
        });
    }

    /// Start a local sound (`startLocalSound`).
    pub fn start_local_sound(&self, sound: Option<PcmSound>, channel: i32) {
        let asset = self.target.borrow().sounds().borrow().asset(&sound);
        let Some(asset) = asset else {
            return;
        };
        let seat = self.target.borrow().seat();
        self.target.borrow_mut().play(PlaySound {
            sound: asset,
            actor: None,
            origin: SoundOrigin::Local,
            seat,
            channel,
            volume: 1.0,
            attenuation: 0.0,
        });
    }

    /// Add a loop sound (`addLoopSound`).
    pub fn add_loop_sound(&self, entity: i32, origin: Vec3, velocity: Vec3, sound: Option<PcmSound>, real_loop: bool) {
        let asset = self.target.borrow().sounds().borrow().asset(&sound);
        let Some(asset) = asset else {
            return;
        };
        let actor = self.target.borrow().actor(entity);
        let seat = self.target.borrow().seat();
        let frame_number = self.target.borrow().frame_number();
        self.target.borrow_mut().loop_sound(LoopSound {
            sound: asset,
            actor,
            origin: SoundOrigin::Fixed { position: origin },
            seat,
            velocity,
            volume: 1.0,
            attenuation: 1.0,
            frame_number,
            persistent: real_loop,
        });
    }

    /// Update a sound position (`updateSoundPosition`).
    pub fn update_sound_position(&self, entity: i32, origin: Vec3) {
        let actor = self.target.borrow().actor(entity);
        self.target.borrow_mut().update_actor(actor, origin);
    }

    /// Stop a looping sound (`stopLoopingSound`).
    pub fn stop_looping_sound(&self, entity: i32) {
        let actor = self.target.borrow().actor(entity);
        let seat = self.target.borrow().seat();
        self.target.borrow_mut().stop_loop(seat, actor);
    }
}
