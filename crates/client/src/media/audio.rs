//! Cinematic audio mixer adapter.
//!
//! Donor provenance: `src/media/audio.ts` (`cinematicAudio`).
//!
//! A movie owns one PCM lane in the shared mixer, regardless of
//! renderer or video format.

use qa_core::identity::SeatId;

use super::types::{AudioSamples, CinematicAudio, CinematicTarget};

/// A cinematic audience (seat or world material).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CinematicAudience {
    /// Seat.
    Seat(SeatId),
    /// World material.
    World,
}

/// Resolve an audience (`target.kind === "seat" ? target : world`).
#[must_use]
pub fn resolve_audience(target: &CinematicTarget) -> CinematicAudience {
    match target {
        CinematicTarget::Seat(seat) => CinematicAudience::Seat(seat.clone()),
        CinematicTarget::Material(_) => CinematicAudience::World,
    }
}

/// A mixer stream target (`AudioStreamTarget`).
#[derive(Debug, Clone, PartialEq)]
pub struct AudioStreamTarget {
    /// Stream id.
    pub id: String,
    /// Audience.
    pub audience: CinematicAudience,
    /// Gain.
    pub gain: f32,
}

/// Streamed PCM (`StreamPcm`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamPcm {
    /// Samples.
    pub samples: AudioSamples,
    /// Channels.
    pub channels: u8,
    /// Sample rate.
    pub sample_rate: u32,
    /// Source sample.
    pub source_sample: usize,
    /// Reset the stream.
    pub reset_stream: bool,
}

impl From<&CinematicAudio> for StreamPcm {
    fn from(audio: &CinematicAudio) -> Self {
        Self {
            samples: audio.samples.clone(),
            channels: audio.channels,
            sample_rate: audio.sample_rate,
            source_sample: audio.source_sample,
            reset_stream: audio.reset_stream,
        }
    }
}

/// A cinematic mixer (`CinematicMixer`).
pub trait CinematicMixer {
    /// Opaque mixer stream checkpoint.
    type StreamCheckpoint: Clone;

    /// Queue a stream.
    fn queue_stream(&mut self, target: &AudioStreamTarget, pcm: &StreamPcm);
    /// Stop a stream.
    fn stop_stream(&mut self, id: &str);
    /// Pause or resume a stream.
    fn pause_stream(&mut self, id: &str, paused: bool);
    /// Capture a stream checkpoint, when the mixer supports it.
    fn capture_stream_checkpoint(&self, _id: &str) -> Option<Self::StreamCheckpoint> {
        None
    }
    /// Restore a stream checkpoint, when the mixer supports it.
    fn restore_stream_checkpoint(&mut self, _target: &AudioStreamTarget, _checkpoint: &Self::StreamCheckpoint) {}
}

/// A cinematic audio adapter (`cinematicAudio`).
pub struct CinematicAudioAdapter<'a, M: CinematicMixer + ?Sized> {
    mixer: &'a mut M,
    id: String,
    gain: f32,
}

/// Build an adapter over one mixer lane (`cinematicAudio`).
pub fn cinematic_audio<'a, M: CinematicMixer + ?Sized>(
    mixer: &'a mut M,
    id: &str,
    gain: f32,
) -> CinematicAudioAdapter<'a, M> {
    CinematicAudioAdapter {
        mixer,
        id: id.to_string(),
        gain,
    }
}

impl<M: CinematicMixer + ?Sized> CinematicAudioAdapter<'_, M> {
    fn target(&self, target: &CinematicTarget) -> AudioStreamTarget {
        AudioStreamTarget {
            id: self.id.clone(),
            gain: self.gain,
            audience: resolve_audience(target),
        }
    }

    /// Handle audio (`onAudio`).
    pub fn on_audio(&mut self, audio: &CinematicAudio, target: &CinematicTarget) {
        let target = self.target(target);
        self.mixer.queue_stream(&target, &StreamPcm::from(audio));
    }

    /// Handle reset (`onAudioReset`).
    pub fn on_audio_reset(&mut self) {
        let id = self.id.clone();
        self.mixer.stop_stream(&id);
    }

    /// Handle pause (`onAudioPause`).
    pub fn on_audio_pause(&mut self, paused: bool) {
        let id = self.id.clone();
        self.mixer.pause_stream(&id, paused);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    struct FixedMixer {
        queued: Vec<(AudioStreamTarget, StreamPcm)>,
        stopped: Vec<String>,
        paused: Vec<(String, bool)>,
    }

    impl CinematicMixer for FixedMixer {
        type StreamCheckpoint = Vec<u8>;

        fn queue_stream(&mut self, target: &AudioStreamTarget, pcm: &StreamPcm) {
            self.queued.push((target.clone(), pcm.clone()));
        }

        fn stop_stream(&mut self, id: &str) {
            self.stopped.push(id.to_string());
        }

        fn pause_stream(&mut self, id: &str, paused: bool) {
            self.paused.push((id.to_string(), paused));
        }
    }

    fn audio() -> CinematicAudio {
        CinematicAudio {
            samples: AudioSamples::U8(vec![1, 2]),
            channels: 1,
            sample_rate: 22050,
            source_sample: 4,
            source_time: 0.0,
            time: 0.0,
            pass: 0,
            reset_stream: true,
        }
    }

    #[test]
    fn routes_one_lane_with_gain() {
        let owner = IdentityOwner::create("test").unwrap();
        let seat = CinematicTarget::Seat(owner.seat(0));
        let material = CinematicTarget::Material("wall".to_string());
        let mut mixer = FixedMixer {
            queued: Vec::new(),
            stopped: Vec::new(),
            paused: Vec::new(),
        };
        let mut adapter = cinematic_audio(&mut mixer, "movie", 0.5);
        adapter.on_audio(&audio(), &seat);
        adapter.on_audio(&audio(), &material);
        adapter.on_audio_pause(true);
        adapter.on_audio_reset();
        assert_eq!(mixer.queued.len(), 2);
        // The reset flag rides through the PCM chunk; the adapter
        // never stops the lane itself.
        assert!(mixer.stopped.iter().any(|id| id == "movie"));
        assert_eq!(mixer.queued[0].0.id, "movie");
        assert_eq!(mixer.queued[0].0.gain, 0.5);
        assert_eq!(mixer.queued[0].0.audience, CinematicAudience::Seat(owner.seat(0)));
        assert_eq!(mixer.queued[1].0.audience, CinematicAudience::World);
        assert_eq!(mixer.queued[0].1.source_sample, 4);
        assert!(mixer.queued[0].1.reset_stream);
        assert_eq!(mixer.paused, vec![("movie".to_string(), true)]);
        assert_eq!(mixer.capture_stream_checkpoint("movie"), None);
    }
}
