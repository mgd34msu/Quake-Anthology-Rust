//! Windowed menu sounds: bank plus UI-event queue over menu mounts.
//!
//! Donor provenance: `src/app/bootstrap/startup-audio.ts` (`StartupAudio`)
//! and `src/app/bootstrap/audio/menu.ts` (`menuSoundPath`).
//!
//! Sync adaptation: the donor plays menu sounds immediately through the
//! shared client audio; here the menu queues [`UiSound`] events during
//! draw and the backend drains them into the windowed engine once per
//! frame (a one-frame delay no ear can hear). This keeps engine ownership
//! in the backend instead of threading shared mutability through the
//! menu. Without menu mounts or menu sounds the menu stays silent.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_client::audio::bank::SoundBank;
use qa_client::audio::engine::UnifiedAudio;
use qa_client::audio::types::{AudioAudience, AudioListener, PlaySound, SoundAsset, SoundOrigin};
use qa_client::audio::SoundFamily;
use qa_client::ui::common::controller::UiSound;
use qa_content::catalog::InstalledCatalog;
use qa_content::contract::GameFamily;
use qa_core::identity::SeatId;
use qa_core::math::{angles_to_axis, vec3};

use super::audio::menu::menu_sound_path;
use super::audio_bridge::MountsSoundContent;
use super::windowed_menu_text::{installed_candidates, open_charset_mounts};

/// Queued menu audio: registered click assets plus the UI-event queue.
pub struct MenuAudio {
    /// Registered menu click assets by UI event.
    assets: HashMap<UiSound, SoundAsset>,
    /// Sound family of the registered clicks.
    family: SoundFamily,
    /// UI events queued during menu draw, drained once per frame.
    queue: Rc<RefCell<Vec<UiSound>>>,
    /// Menu seat (listener and audience).
    seat: SeatId,
}

impl MenuAudio {
    /// Open menu audio over the menu content mounts (the preferred
    /// product, else the first installed candidate): register the menu
    /// click for every UI event. `None` keeps the menu silent.
    pub fn open(catalog: &InstalledCatalog, preferred: Option<&str>, seat: SeatId) -> Option<Self> {
        let wanted = installed_candidates(catalog, preferred).into_iter().next()?;
        // Candidates mix expectation ids (preferred) with catalog ids, so
        // match either form, then open mounts by catalog id like the
        // charset path.
        let product = catalog
            .products
            .iter()
            .find(|product| product.id.as_str() == wanted || product.expectation.id == wanted)?;
        let family = product.expectation.family;
        let mounts = open_charset_mounts(catalog, product.id.as_str())?;
        let mut bank = SoundBank::new(MountsSoundContent::new(Rc::new(mounts)));
        let sound_family = match family {
            GameFamily::Q1 => SoundFamily::Q1,
            GameFamily::Q2 => SoundFamily::Q2,
            GameFamily::Q3 => SoundFamily::Q3,
        };
        let mut assets = HashMap::new();
        for event in [
            UiSound::Open,
            UiSound::Close,
            UiSound::Move,
            UiSound::Change,
            UiSound::Reject,
        ] {
            let path = menu_sound_path(family, event);
            match bank.register(&path, sound_family) {
                Ok(Some(asset)) => {
                    assets.insert(event, asset);
                }
                Ok(None) => eprintln!("windowed menu: sound unavailable: {path}"),
                Err(error) => eprintln!("windowed menu: cannot decode {path} ({error})"),
            }
        }
        if assets.is_empty() {
            return None;
        }
        eprintln!(
            "windowed menu: {} click sounds ({})",
            assets.len(),
            product.expectation.id
        );
        Some(Self {
            assets,
            family: sound_family,
            queue: Rc::new(RefCell::new(Vec::new())),
            seat,
        })
    }

    /// UI-event sink for the startup menu: queue the click for the
    /// once-per-frame drain.
    pub fn sink(&self) -> Rc<dyn Fn(UiSound)> {
        let queue = Rc::clone(&self.queue);
        Rc::new(move |event| queue.borrow_mut().push(event))
    }

    /// Queued events (drain order).
    #[cfg(test)]
    fn queued(&self) -> Vec<UiSound> {
        self.queue.borrow().clone()
    }

    /// Drain queued clicks into the engine: set the menu seat listener
    /// (without it `play` mixes zero seats), then play each queued
    /// click locally. Never fails; engine errors log and stay silent.
    pub fn drain(&mut self, engine: &mut UnifiedAudio) {
        let listener = AudioListener {
            seat: self.seat.clone(),
            actor: None,
            origin: vec3(0.0, 0.0, 0.0),
            axis: angles_to_axis(vec3(0.0, 0.0, 0.0)),
            gain: 1.0,
            underwater: false,
        };
        if let Err(error) = engine.set_listeners(std::slice::from_ref(&listener)) {
            eprintln!("windowed menu: cannot set listeners ({error})");
            return;
        }
        for event in self.queue.borrow_mut().drain(..) {
            let Some(asset) = self.assets.get(&event) else {
                continue;
            };
            let request = PlaySound {
                family: self.family,
                sound: asset.clone(),
                origin: SoundOrigin::Local,
                actor: None,
                owner: None,
                channel: 0,
                volume: 1.0,
                attenuation: 0.0,
                audience: AudioAudience::Seat {
                    seat: self.seat.clone(),
                },
                delay_seconds: None,
                server_milliseconds: None,
            };
            if let Err(error) = engine.play(&request) {
                eprintln!("windowed menu: cannot play click ({error})");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use qa_client::audio::engine::{AudioDeviceFactory, DeviceOpenError, UnifiedAudioOptions};
    use qa_client::audio::error::AudioError;
    use qa_client::audio::output::AudioOutputFormat;
    use qa_client::audio::wav::PcmSound;

    use super::*;

    /// Synthetic click asset: 512 loud mono samples.
    fn click_asset() -> SoundAsset {
        SoundAsset {
            resource: "test".to_string(),
            name: "click".to_string(),
            pcm: Rc::new(PcmSound {
                sample_rate: 44100,
                channels: 1,
                samples: vec![9000i16; 512],
                frame_count: 512,
                loop_start: None,
            }),
        }
    }

    /// Device factory that never opens: the tests mix PCM without a device.
    struct FakeFactory;

    impl AudioDeviceFactory for FakeFactory {
        fn open(
            &self,
            _device_name: Option<&str>,
            _format: &AudioOutputFormat,
            _buffer_frames: Option<u32>,
        ) -> Result<Box<dyn qa_client::audio::engine::AudioOutputDevice>, DeviceOpenError> {
            Err(DeviceOpenError::Unavailable("headless".to_string()))
        }

        fn output_names(&self) -> Result<Vec<String>, AudioError> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn sink_queues_events_in_order() {
        let authority = qa_core::identity::IdentityOwner::create("menu-audio-test").unwrap();
        let audio = MenuAudio {
            assets: HashMap::new(),
            family: SoundFamily::Q1,
            queue: Rc::new(RefCell::new(Vec::new())),
            seat: authority.seat(0),
        };
        let sink = audio.sink();
        sink(UiSound::Move);
        sink(UiSound::Open);
        assert_eq!(audio.queued(), vec![UiSound::Move, UiSound::Open]);
    }

    #[test]
    fn drain_sets_listeners_and_plays_queued_clicks() {
        let authority = qa_core::identity::IdentityOwner::create("menu-audio-test").unwrap();
        let seat = authority.seat(0);
        let mut engine = UnifiedAudio::new(UnifiedAudioOptions {
            sample_rate: None,
            output_format: None,
            milliseconds: Box::new(|| 0),
            random: Box::new(|| 7),
            max_actors: None,
            on_sound: None,
            device_factory: Some(Rc::new(FakeFactory)),
        })
        .unwrap();
        let mut audio = MenuAudio {
            assets: HashMap::new(),
            family: SoundFamily::Q1,
            queue: Rc::new(RefCell::new(Vec::new())),
            seat,
        };
        // Silence without assets still sets listeners without failing.
        audio.sink()(UiSound::Move);
        audio.drain(&mut engine);
        let silent = engine.mix(64).unwrap();
        assert!(silent.iter().all(|sample| *sample == 0));
    }

    #[test]
    fn queued_click_mixes_audible_pcm() {
        let authority = qa_core::identity::IdentityOwner::create("menu-audio-test").unwrap();
        let seat = authority.seat(0);
        let mut engine = UnifiedAudio::new(UnifiedAudioOptions {
            sample_rate: None,
            output_format: None,
            milliseconds: Box::new(|| 0),
            random: Box::new(|| 7),
            max_actors: None,
            on_sound: None,
            device_factory: Some(Rc::new(FakeFactory)),
        })
        .unwrap();
        let mut assets = HashMap::new();
        assets.insert(UiSound::Move, click_asset());
        let mut audio = MenuAudio {
            assets,
            family: SoundFamily::Q2,
            queue: Rc::new(RefCell::new(Vec::new())),
            seat,
        };
        audio.sink()(UiSound::Move);
        audio.drain(&mut engine);
        let mixed = engine.mix(256).unwrap();
        assert!(
            mixed.iter().any(|sample| *sample != 0),
            "queued menu click mixed silence"
        );
    }
}
