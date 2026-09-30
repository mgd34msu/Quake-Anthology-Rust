//! Quake III presentation: frame audio.
//!
//! Donor provenance: `src/content/q3/presentation/frame-audio.ts`.

use qa_core::math::Vec3;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::hud::Shared;
use crate::q3::presentation::retail_snapshot::PcmSound;
use crate::q3::presentation::state::*;

/// Frame audio host (`ClientFrameAudioHost`).
pub trait ClientFrameAudioHost {
    /// Start a local sound.
    fn start_local_sound(&mut self, sound: Option<PcmSound>, channel: i32);
    /// Start a placed sound.
    fn start_sound(&mut self, origin: Option<Vec3>, entity: i32, channel: i32, sound: Option<PcmSound>);
}

/// Buffered and powerup audio (`ClientFrameAudio`).
pub struct ClientFrameAudio {
    /// Frame state.
    pub state: Shared<ClientGameState>,
    /// Wear-off sound.
    pub wear_off_sound: Option<PcmSound>,
    /// Host.
    pub host: Shared<dyn ClientFrameAudioHost>,
}

impl ClientFrameAudio {
    /// Assemble frame audio.
    pub fn new(
        state: Shared<ClientGameState>,
        wear_off_sound: Option<PcmSound>,
        host: Shared<dyn ClientFrameAudioHost>,
    ) -> Self {
        Self {
            state,
            wear_off_sound,
            host,
        }
    }

    /// Buffer a sound (`addBufferedSound`).
    pub fn add_buffered_sound(&self, sound: Option<PcmSound>) {
        let Some(sound) = sound else {
            return;
        };
        let mut state = self.state.borrow_mut();
        let input = state.sound_buffer_in;
        state.sound_buffer[input as usize % 20] = Some(sound);
        state.sound_buffer_in = (input + 1) % 20;
        if state.sound_buffer_in == state.sound_buffer_out {
            state.sound_buffer_out += 1;
        }
    }

    /// Play buffered sounds (`playBufferedSounds`).
    pub fn play_buffered_sounds(&self) {
        let (sound_time, time, output, input) = {
            let state = self.state.borrow();
            (
                state.sound_time,
                state.time,
                state.sound_buffer_out,
                state.sound_buffer_in,
            )
        };
        if sound_time >= time || output == input {
            return;
        }
        let sound = self
            .state
            .borrow()
            .sound_buffer
            .get(output as usize)
            .cloned()
            .unwrap_or_else(|| panic!("CG_PlayBufferedSounds: sound buffer index {output} outside 0..19"));
        let Some(sound) = sound else {
            return;
        };
        self.host.borrow_mut().start_local_sound(Some(sound), 7);
        let mut state = self.state.borrow_mut();
        state.sound_buffer[output as usize] = None;
        state.sound_buffer_out = (output + 1) % 20;
        state.sound_time = state.time.wrapping_add(750);
    }

    /// Powerup timer sounds (`powerupTimerSounds`).
    pub fn powerup_timer_sounds(&self) {
        let snapshot = self.state.borrow().snap.clone();
        let Some(snapshot) = snapshot else {
            panic!("CG_PowerupTimerSounds requires an active snapshot");
        };
        let (time, old_time) = {
            let state = self.state.borrow();
            (state.time, state.old_time)
        };
        for slot in 0..16 {
            let expiry = snapshot.player_state.powerups.get(slot);
            if expiry <= time {
                continue;
            }
            let remaining = expiry.wrapping_sub(time);
            if remaining >= 5000 {
                continue;
            }
            let previous = expiry.wrapping_sub(old_time);
            if remaining / 1000 != previous / 1000 {
                self.host
                    .borrow_mut()
                    .start_sound(None, snapshot.player_state.client_num, 4, self.wear_off_sound);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q3::base::shared::definitions::{Powerup, Product};
    use crate::q3::base::shared::player_state::PlayerState;
    use crate::q3::presentation::hud::tests::*;
    use crate::q3::presentation::hud::{shared, Shared};
    use crate::q3::presentation::retail_snapshot::{PcmSound as RetailPcmSound, Snapshot};
    use crate::q3::presentation::state::ClientGameState;

    #[test]
    fn frame_audio_buffering() {
        let state = shared(ClientGameState::new(Product::Baseq3, 0, 0).unwrap());
        let host: Shared<FakeFrameAudio> = shared(FakeFrameAudio::default());
        let audio = ClientFrameAudio::new(state.clone(), Some(RetailPcmSound::new(1)), host.clone());
        audio.add_buffered_sound(Some(RetailPcmSound::new(2)));
        state.borrow_mut().time = 100;
        audio.play_buffered_sounds();
        assert_eq!(host.borrow().local, vec![7]);
        assert_eq!(state.borrow().sound_time, 850);
        audio.play_buffered_sounds();
        assert_eq!(host.borrow().local.len(), 1);
    }

    #[test]
    fn frame_audio_powerup_tick() {
        let state = shared(ClientGameState::new(Product::Baseq3, 0, 0).unwrap());
        let mut ps = PlayerState::new(Product::Baseq3, None);
        ps.powerups.set(Powerup::PwQuad as usize, 4500);
        state.borrow_mut().snap = Some(Snapshot {
            message_number: 0,
            server_time: 3000,
            delta_number: 0,
            flags: 0,
            server_command_number: 0,
            parse_entities_number: 0,
            area_mask: [0; 32],
            player_state: ps,
            entities: Vec::new(),
        });
        state.borrow_mut().time = 3000;
        state.borrow_mut().old_time = 1500;
        let host: Shared<FakeFrameAudio> = shared(FakeFrameAudio::default());
        ClientFrameAudio::new(state, Some(RetailPcmSound::new(1)), host.clone()).powerup_timer_sounds();
        assert_eq!(host.borrow().placed.len(), 1);
    }
}
