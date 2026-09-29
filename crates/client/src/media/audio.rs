//! Cinematic audio mixer adapter.
//!
//! Donor provenance: `src/media/audio.ts` (`cinematicAudio`).

use qa_core::identity::SeatId;

use super::types::{AudioSamples, CinematicAudio, CinematicTarget};

/// A cinematic mixer (`audioMixer` subset, sync).
pub trait CinematicMixer {
    /// Queue a stream.
    fn queue_stream(
        &mut self,
        audience: &CinematicAudience,
        samples: &AudioSamples,
        sample_rate: u32,
    );
    /// Stop a stream.
    fn stop_stream(&mut self, audience: &CinematicAudience);
    /// Pause or resume a stream.
    fn pause_stream(&mut self, audience: &CinematicAudience, paused: bool);
}

/// A cinematic audience (seat or world material).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CinematicAudience {
    /// Seat.
    Seat(SeatId),
    /// World material.
    World,
}

/// Resolve an audience (`resolveAudience`).
#[must_use]
pub fn resolve_audience(target: &CinematicTarget) -> CinematicAudience {
    match target {
        CinematicTarget::Seat(seat) => CinematicAudience::Seat(seat.clone()),
        CinematicTarget::Material(_) => CinematicAudience::World,
    }
}

/// A cinematic audio adapter (`cinematicAudio`).
pub struct CinematicAudioAdapter<'a> {
    mixer: &'a mut dyn CinematicMixer,
}

impl<'a> CinematicAudioAdapter<'a> {
    /// New adapter.
    #[must_use]
    pub fn new(mixer: &'a mut dyn CinematicMixer) -> Self {
        Self { mixer }
    }

    /// Handle audio.
    pub fn on_audio(&mut self, audio: &CinematicAudio, target: &CinematicTarget) {
        if audio.reset_stream {
            self.mixer.stop_stream(&resolve_audience(target));
        }
        self.mixer.queue_stream(
            &resolve_audience(target),
            &audio.samples,
            audio.sample_rate,
        );
    }

    /// Handle reset.
    pub fn on_audio_reset(&mut self, target: &CinematicTarget) {
        self.mixer.stop_stream(&resolve_audience(target));
    }

    /// Handle pause.
    pub fn on_audio_pause(&mut self, paused: bool, target: &CinematicTarget) {
        self.mixer.pause_stream(&resolve_audience(target), paused);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    struct FixedMixer {
        queued: usize,
        stopped: usize,
        paused: Vec<bool>,
    }

    impl CinematicMixer for FixedMixer {
        fn queue_stream(
            &mut self,
            _audience: &CinematicAudience,
            _samples: &AudioSamples,
            _sample_rate: u32,
        ) {
            self.queued += 1;
        }

        fn stop_stream(&mut self, _audience: &CinematicAudience) {
            self.stopped += 1;
        }

        fn pause_stream(&mut self, _audience: &CinematicAudience, paused: bool) {
            self.paused.push(paused);
        }
    }

    #[test]
    fn reset_stops_before_queue() {
        let owner = IdentityOwner::create("test").unwrap();
        let target = CinematicTarget::Seat(owner.seat(0));
        let mut mixer = FixedMixer {
            queued: 0,
            stopped: 0,
            paused: Vec::new(),
        };
        let mut adapter = CinematicAudioAdapter::new(&mut mixer);
        adapter.on_audio(
            &CinematicAudio {
                samples: AudioSamples::U8(vec![1, 2]),
                channels: 1,
                sample_rate: 22050,
                source_sample: 0,
                source_time: 0.0,
                time: 0.0,
                pass: 0,
                reset_stream: true,
            },
            &target,
        );
        adapter.on_audio_pause(true, &target);
        assert_eq!(mixer.queued, 1);
        assert_eq!(mixer.stopped, 1);
        assert_eq!(mixer.paused, vec![true]);
        assert_eq!(
            resolve_audience(&target),
            CinematicAudience::Seat(owner.seat(0))
        );
    }
}
