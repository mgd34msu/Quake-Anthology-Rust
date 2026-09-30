//! Intro/loop music and CD track selection.
//!
//! Donor provenance: `src/audio/music.ts` (`MusicControls`,
//! `MusicPlayer`, `CdMusic`, `remapQ2MusicTrack`, from Q3 `snd_dma.c`
//! and Q1/Q2 `cd_ogg.ts`).

use super::error::AudioError;
use super::streams::{PcmStream, RawAudioStream};
use super::types::{StreamPcm, StreamSamples};
use crate::audio::SoundFamily;

/// Music volume mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MusicVolumeMode {
    /// Q3 smoothed source volume.
    Source,
    /// Immediate target volume.
    Immediate,
}

/// CD controls: enable flag plus track remapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MusicControls {
    /// Music enabled.
    pub enabled: bool,
    remap: [i32; 100],
}

impl MusicControls {
    /// Fresh identity controls.
    #[must_use]
    pub fn new() -> Self {
        Self {
            enabled: true,
            remap: std::array::from_fn(|index| index as i32),
        }
    }

    /// Remapped tracks 1..=99.
    #[must_use]
    pub fn remapped_tracks(&self) -> Vec<i32> {
        self.remap[1..].to_vec()
    }

    /// Map a requested track.
    #[must_use]
    pub fn mapped_track(&self, track: i32) -> i32 {
        usize::try_from(track)
            .ok()
            .and_then(|index| self.remap.get(index).copied())
            .unwrap_or(track)
    }

    /// Replace the remap table.
    pub fn set_remap(&mut self, tracks: &[i32]) -> Result<(), AudioError> {
        if tracks.len() >= self.remap.len() {
            return Err(AudioError::CdRemap);
        }
        for track in tracks {
            if !(0..=255).contains(track) {
                return Err(AudioError::BadCdTrack);
            }
        }
        for (index, track) in tracks.iter().enumerate() {
            self.remap[index + 1] = *track;
        }
        Ok(())
    }

    /// Restore identity controls.
    pub fn reset(&mut self) {
        self.enabled = true;
        for (index, slot) in self.remap.iter_mut().enumerate() {
            *slot = index as i32;
        }
    }
}

impl Default for MusicControls {
    fn default() -> Self {
        Self::new()
    }
}

/// Loop source for [`MusicPlayer::start`]: the intro itself or a stream.
pub enum LoopSource {
    /// Loop the intro stream.
    SameAsIntro,
    /// Loop a separate stream.
    Stream(Box<dyn PcmStream>),
}

enum LoopState {
    Same,
    Other(Box<dyn PcmStream>),
}

/// Intro/loop music player over the stream bus.
pub struct MusicPlayer {
    stream: Option<Box<dyn PcmStream>>,
    loop_stream: Option<LoopState>,
    pcm: RawAudioStream,
    target_volume: f64,
    smoothed_volume: f32,
    /// Paused players mix silence.
    pub paused: bool,
    completions: u64,
    output_rate: u32,
    family: SoundFamily,
    volume_mode: MusicVolumeMode,
    /// Shared controls.
    pub controls: MusicControls,
}

impl MusicPlayer {
    /// Player at an output rate.
    #[must_use]
    pub fn new(output_rate: u32, family: SoundFamily, volume_mode: MusicVolumeMode, controls: MusicControls) -> Self {
        Self {
            stream: None,
            loop_stream: None,
            pcm: RawAudioStream::new(output_rate),
            target_volume: 0.25,
            smoothed_volume: 0.5,
            paused: false,
            completions: 0,
            output_rate,
            family,
            volume_mode,
            controls,
        }
    }

    /// Output rate in Hz.
    #[must_use]
    pub const fn output_rate(&self) -> u32 {
        self.output_rate
    }

    /// Whether a stream is playing.
    #[must_use]
    pub const fn playing(&self) -> bool {
        self.stream.is_some()
    }

    /// Completed intro plays.
    #[must_use]
    pub const fn completed_plays(&self) -> u64 {
        self.completions
    }

    /// Stream bus source position.
    #[must_use]
    pub fn source_position(&self) -> i64 {
        self.pcm.source_position()
    }

    /// Current volume.
    #[must_use]
    pub fn volume(&self) -> f64 {
        if self.family == SoundFamily::Q3 && self.volume_mode == MusicVolumeMode::Source {
            f64::from(self.smoothed_volume)
        } else {
            self.target_volume
        }
    }

    /// Set the target volume.
    pub fn set_volume(&mut self, value: f64) -> Result<(), AudioError> {
        if !value.is_finite() || value < 0.0 {
            return Err(AudioError::BadMusicVolume);
        }
        self.target_volume = value;
        Ok(())
    }

    /// Advance source smoothing once per presentation frame.
    pub fn update(&mut self) {
        if self.family == SoundFamily::Q3
            && self.volume_mode == MusicVolumeMode::Source
            && self.stream.is_some()
            && !self.paused
            && self.controls.enabled
        {
            self.smoothed_volume = (self.smoothed_volume + (self.target_volume as f32) * 2.0) / 4.0;
        }
    }

    /// Start an intro with an optional loop.
    pub fn start(&mut self, intro: Box<dyn PcmStream>, loop_source: Option<LoopSource>) {
        self.stop();
        self.stream = Some(intro);
        self.loop_stream = loop_source.map(|source| match source {
            LoopSource::SameAsIntro => LoopState::Same,
            LoopSource::Stream(stream) => LoopState::Other(stream),
        });
        self.pcm = RawAudioStream::new(self.output_rate);
        self.paused = false;
    }

    /// Stop and close all streams.
    pub fn stop(&mut self) {
        if let Some(mut stream) = self.stream.take() {
            stream.close();
        }
        if let Some(LoopState::Other(mut stream)) = self.loop_stream.take() {
            stream.close();
        }
    }

    fn next_chunk(&mut self) -> Result<Option<StreamPcm>, AudioError> {
        let Some(stream) = self.stream.as_mut() else {
            return Ok(None);
        };
        let source_sample = stream.position_frames()?;
        if let Some(chunk) = stream.read(16384)? {
            return Ok(Some(StreamPcm {
                samples: StreamSamples::S16(chunk.samples),
                sample_rate: chunk.sample_rate,
                channels: chunk.channels,
                source_sample: source_sample as i64,
                reset_stream: false,
            }));
        }
        let loop_state = self.loop_stream.take();
        match loop_state {
            None => {
                self.completions += 1;
                if let Some(mut stream) = self.stream.take() {
                    stream.close();
                }
                Ok(None)
            }
            Some(LoopState::Same) => {
                self.loop_stream = Some(LoopState::Same);
                let Some(stream) = self.stream.as_mut() else {
                    self.stop();
                    return Ok(None);
                };
                stream.seek(0)?;
                let Some(chunk) = stream.read(16384)? else {
                    self.stop();
                    return Ok(None);
                };
                Ok(Some(StreamPcm {
                    samples: StreamSamples::S16(chunk.samples),
                    sample_rate: chunk.sample_rate,
                    channels: chunk.channels,
                    source_sample: 0,
                    reset_stream: true,
                }))
            }
            Some(LoopState::Other(mut other)) => {
                if let Some(mut stream) = self.stream.take() {
                    stream.close();
                }
                other.seek(0)?;
                let chunk = other.read(16384)?;
                self.stream = Some(other);
                let Some(chunk) = chunk else {
                    self.stop();
                    return Ok(None);
                };
                Ok(Some(StreamPcm {
                    samples: StreamSamples::S16(chunk.samples),
                    sample_rate: chunk.sample_rate,
                    channels: chunk.channels,
                    source_sample: 0,
                    reset_stream: true,
                }))
            }
        }
    }

    /// Mix frames through the stream bus.
    pub fn mix(&mut self, frames: usize) -> Result<Vec<f64>, AudioError> {
        if !self.controls.enabled || self.paused || self.volume() <= 0.0 || self.stream.is_none() {
            return Ok(vec![0.0; frames * 2]);
        }
        let volume = self.volume();
        let mut pcm = std::mem::replace(&mut self.pcm, RawAudioStream::new(self.output_rate));
        let result = pcm.mix(frames, volume, Some(&mut || self.next_chunk()));
        self.pcm = pcm;
        result
    }

    /// Stop the player.
    pub fn close(&mut self) {
        self.stop();
    }
}

/// Open a music track path.
pub type OpenMusicTrack = Box<dyn FnMut(&str) -> Result<Option<Box<dyn PcmStream>>, AudioError>>;

/// CD track selection over a player.
pub struct CdMusic {
    player: MusicPlayer,
    open: OpenMusicTrack,
    track: Option<i32>,
    request: u64,
}

impl CdMusic {
    /// CD selection over a player.
    #[must_use]
    pub fn new(player: MusicPlayer, open: OpenMusicTrack) -> Self {
        Self {
            player,
            open,
            track: None,
            request: 0,
        }
    }

    /// Borrow the driven player (donor `CdMusic.player` is a public field).
    #[must_use]
    pub const fn player(&self) -> &MusicPlayer {
        &self.player
    }

    /// Mutably borrow the driven player.
    pub fn player_mut(&mut self) -> &mut MusicPlayer {
        &mut self.player
    }

    /// Whether music is enabled.
    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.player.controls.enabled
    }

    /// Enable or disable music.
    pub fn set_enabled(&mut self, value: bool) {
        self.player.controls.enabled = value;
    }

    /// Playing track, if any.
    #[must_use]
    pub const fn playing_track(&self) -> Option<i32> {
        self.track
    }

    /// Remapped tracks.
    #[must_use]
    pub fn remapped_tracks(&self) -> Vec<i32> {
        self.player.controls.remapped_tracks()
    }

    /// Replace the remap table.
    pub fn set_remap(&mut self, tracks: &[i32]) -> Result<(), AudioError> {
        self.player.controls.set_remap(tracks)
    }

    /// Stop and reset controls.
    pub fn reset(&mut self) {
        self.stop();
        self.player.controls.reset();
    }

    /// Play a track, looping on one stream when asked.
    pub fn play(&mut self, track: i32, looping: bool) -> Result<bool, AudioError> {
        if !self.enabled() {
            return Ok(false);
        }
        if !(0..=255).contains(&track) {
            return Err(AudioError::BadCdTrack);
        }
        let mapped = self.player.controls.mapped_track(track);
        if mapped < 1 {
            return Ok(false);
        }
        if self.track == Some(mapped) && self.player.playing() {
            return Ok(true);
        }
        let request = self.request + 1;
        self.request = request;
        self.player.stop();
        self.track = None;
        let number = format!("{mapped:02}");
        for path in [
            format!("music/{number}.ogg"),
            format!("music/track{number}.ogg"),
            format!("music/{number}.wav"),
            format!("music/track{number}.wav"),
        ] {
            let stream = (self.open)(&path)?;
            if request != self.request || !self.enabled() {
                if let Some(mut stream) = stream {
                    stream.close();
                }
                return Ok(false);
            }
            if let Some(stream) = stream {
                self.player.start(stream, looping.then_some(LoopSource::SameAsIntro));
                self.track = Some(mapped);
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Pause the player.
    pub fn pause(&mut self) {
        self.player.paused = true;
    }

    /// Resume the player when enabled.
    pub fn resume(&mut self) {
        if self.enabled() {
            self.player.paused = false;
        }
    }

    /// Invalidate a pending open.
    pub fn invalidate_pending(&mut self) {
        self.request += 1;
    }

    /// Stop playback.
    pub fn stop(&mut self) {
        self.request += 1;
        self.player.stop();
        self.track = None;
    }

    /// Close CD music.
    pub fn close(&mut self) {
        self.stop();
    }
}

const XATRIX_TRACKS: [i32; 10] = [9, 13, 14, 7, 16, 2, 15, 3, 4, 18];

/// Q2 soundtrack profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q2SoundtrackProfile {
    /// Original disc order.
    Disc,
    /// Remastered order for a campaign.
    Remastered {
        /// Campaign name.
        campaign: String,
    },
}

/// Remap a Q2 track for the soundtrack profile.
pub fn remap_q2_music_track(track: i32, profile: &Q2SoundtrackProfile) -> Result<i32, AudioError> {
    let Q2SoundtrackProfile::Remastered { campaign } = profile else {
        return Ok(track);
    };
    if !(2..=11).contains(&track) {
        return Ok(track);
    }
    let game = campaign.to_lowercase();
    if game == "rogue" {
        return Ok(track + 10);
    }
    if game == "xatrix" {
        return XATRIX_TRACKS
            .get((track - 2) as usize)
            .copied()
            .ok_or(AudioError::XatrixTrack);
    }
    Ok(track)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::streams::MemoryPcmStream;
    use crate::audio::wav::PcmSound;

    fn pcm(samples: Vec<i16>) -> PcmSound {
        PcmSound {
            sample_rate: 11025,
            channels: 1,
            frame_count: samples.len(),
            samples,
            loop_start: None,
        }
    }

    #[test]
    fn loops_intro_and_counts_completions() {
        let mut player = MusicPlayer::new(11025, SoundFamily::Q3, MusicVolumeMode::Immediate, MusicControls::new());
        player.set_volume(1.0).unwrap();
        player.start(
            Box::new(MemoryPcmStream::new(pcm(vec![1000, 2000]))),
            Some(LoopSource::SameAsIntro),
        );
        assert!(player.playing());
        let mixed = player.mix(4).unwrap();
        assert_eq!(
            mixed,
            vec![1000.0, 1000.0, 2000.0, 2000.0, 1000.0, 1000.0, 2000.0, 2000.0]
        );
        // The loop chunk resets the stream, so the position restarts at zero.
        assert_eq!(player.source_position(), 2);
        player.stop();
        player.start(Box::new(MemoryPcmStream::new(pcm(vec![1000]))), None);
        let mixed = player.mix(2).unwrap();
        assert_eq!(mixed, vec![1000.0, 1000.0, 0.0, 0.0]);
        assert_eq!(player.completed_plays(), 1);
        assert!(!player.playing());
    }

    #[test]
    fn smoothing_and_cd_selection() {
        let mut player = MusicPlayer::new(11025, SoundFamily::Q3, MusicVolumeMode::Source, MusicControls::new());
        player.start(Box::new(MemoryPcmStream::new(pcm(vec![0]))), None);
        player.set_volume(1.0).unwrap();
        player.update();
        assert!((player.volume() - 0.625).abs() < 1e-6);
        let mut cd = CdMusic::new(
            MusicPlayer::new(11025, SoundFamily::Q2, MusicVolumeMode::Immediate, MusicControls::new()),
            Box::new(|path| {
                Ok(path
                    .ends_with("02.ogg")
                    .then(|| Box::new(MemoryPcmStream::new(pcm(vec![7]))) as Box<dyn PcmStream>))
            }),
        );
        assert!(cd.play(2, true).unwrap());
        assert_eq!(cd.playing_track(), Some(2));
        assert!(cd.play(2, true).unwrap());
        assert!(!cd.play(3, false).unwrap());
        assert!(cd.play(99, false).is_ok());
        assert_eq!(remap_q2_music_track(2, &Q2SoundtrackProfile::Disc).unwrap(), 2);
        assert_eq!(
            remap_q2_music_track(
                2,
                &Q2SoundtrackProfile::Remastered {
                    campaign: "rogue".to_string()
                }
            )
            .unwrap(),
            12
        );
        assert_eq!(
            remap_q2_music_track(
                2,
                &Q2SoundtrackProfile::Remastered {
                    campaign: "xatrix".to_string()
                }
            )
            .unwrap(),
            9
        );
    }
}
