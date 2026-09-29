//! Audio errors shared by the client audio modules.
//!
//! `crates/client/src/error.rs` belongs to another lane, so audio
//! defines its own error type here. Messages mirror the donor
//! `src/audio/*` throws.

use thiserror::Error;

/// Audio error.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AudioError {
    /// Audio output requires 8000–192000 Hz, 1 or 2 channels, and 8 or 16 bits.
    #[error("Audio output requires 8000–192000 Hz, 1 or 2 channels, and 8 or 16 bits")]
    BadOutputFormat,
    /// Output PCM must contain stereo frames.
    #[error("Output PCM must contain stereo frames")]
    StereoFrames,
    /// Invalid audio output rate.
    #[error("Invalid audio output rate")]
    BadOutputRate,
    /// Audio output channel count must be one or two.
    #[error("Audio output channel count must be one or two")]
    BadOutputChannels,
    /// Audio output sample bits must be 8 or 16.
    #[error("Audio output sample bits must be 8 or 16")]
    BadOutputBits,
    /// Mix request exceeds two seconds.
    #[error("Mix request exceeds two seconds")]
    MixTooLarge,
    /// Audio engine closed.
    #[error("Audio engine closed")]
    Closed,
    /// Audio output already open.
    #[error("Audio output already open")]
    AlreadyOpen,
    /// Audio output is not open.
    #[error("Audio output is not open")]
    NotOpen,
    /// Replacement audio already owns an output.
    #[error("Replacement audio already owns an output")]
    ReplacementOwnsOutput,
    /// Replacement audio requires the same device sample rate.
    #[error("Replacement audio requires the same device sample rate")]
    ReplacementRateMismatch,
    /// Invalid audio lookahead.
    #[error("Invalid audio lookahead")]
    BadLookahead,
    /// Invalid measured audio frame work.
    #[error("Invalid measured audio frame work")]
    BadMeasuredWork,
    /// Audio output selection and recovery failed.
    #[error("Audio output selection and recovery failed: {0}")]
    OutputSelectionFailed(String),
    /// Sound volume must be 0..1.
    #[error("Sound volume must be 0..1")]
    BadVolume,
    /// Invalid effects gain.
    #[error("Invalid effects gain")]
    BadEffectsGain,
    /// Invalid listener gain.
    #[error("Invalid listener gain")]
    BadListenerGain,
    /// Duplicate audio seat.
    #[error("Duplicate audio seat")]
    DuplicateSeat,
    /// Unknown audio seat.
    #[error("Unknown audio seat")]
    UnknownSeat,
    /// Missing listener.
    #[error("Missing listener")]
    MissingListener,
    /// Audio actor capacity exhausted.
    #[error("Audio actor capacity exhausted")]
    ActorCapacity,
    /// Loop requires an actor position.
    #[error("Loop requires an actor position")]
    LoopPosition,
    /// Music output rate differs from device mix.
    #[error("Music output rate differs from device mix")]
    MusicRateMismatch,
    /// PCM restore requires an unused stream lane.
    #[error("PCM restore requires an unused stream lane")]
    StreamLaneUsed,
    /// Audio bus length mismatch.
    #[error("Audio bus length mismatch")]
    BusLength,
    /// {name} must be a positive safe integer.
    #[error("{name} must be a positive safe integer")]
    NotPositiveInteger {
        /// Setting name.
        name: String,
    },
    /// entity must be an integer from 0 through {limit}.
    #[error("entity must be an integer from 0 through {limit}")]
    BadEntity {
        /// Exclusive limit minus one.
        limit: String,
    },
    /// channel must be a signed 32-bit integer.
    #[error("channel must be a signed 32-bit integer")]
    BadChannel,
    /// Invalid source sound channel.
    #[error("Invalid source sound channel")]
    BadSourceChannel,
    /// Raw sample access outside allocation at {0}.
    #[error("Raw sample access outside allocation at {0}")]
    RawAccess(usize),
    /// frame number must be a signed 32-bit integer.
    #[error("frame number must be a signed 32-bit integer")]
    BadFrameNumber,
    /// channel volume must be an integer from 0 through 255.
    #[error("channel volume must be an integer from 0 through 255")]
    BadChannelVolume,
    /// {name} must be a finite nonnegative number.
    #[error("{name} must be a finite nonnegative number")]
    BadGain {
        /// Setting name.
        name: String,
    },
    /// PCM sample index {index} is outside {length} samples.
    #[error("PCM sample index {index} is outside {length} samples")]
    BadSampleIndex {
        /// Index.
        index: String,
        /// Length.
        length: String,
    },
    /// paint index {index} is outside {length} values.
    #[error("paint index {index} is outside {length} values")]
    BadPaintIndex {
        /// Index.
        index: String,
        /// Length.
        length: String,
    },
    /// raw sample index {index} is outside {length} values.
    #[error("raw sample index {index} is outside {length} values")]
    BadRawIndex {
        /// Index.
        index: String,
        /// Length.
        length: String,
    },
    /// PCM frame count must be a nonnegative safe integer.
    #[error("PCM frame count must be a nonnegative safe integer")]
    BadFrameCount,
    /// PCM channel count must be one or two.
    #[error("PCM channel count must be one or two")]
    BadPcmChannels,
    /// sound effects must be mono, matching Quake S_LoadSound.
    #[error("sound effects must be mono, matching Quake S_LoadSound")]
    EffectNotMono,
    /// PCM has {have} samples, expected {want}.
    #[error("PCM has {have} samples, expected {want}")]
    BadSampleCount {
        /// Actual.
        have: String,
        /// Expected.
        want: String,
    },
    /// PCM loop start is outside the sound.
    #[error("PCM loop start is outside the sound")]
    BadLoopStart,
    /// sound time must advance monotonically and paint time must be a safe integer.
    #[error("sound time must advance monotonically and paint time must be a safe integer")]
    BadTimeSelect,
    /// sound epoch rebase requires advancing delivery beyond a complete epoch.
    #[error("sound epoch rebase requires advancing delivery beyond a complete epoch")]
    BadRebase,
    /// rebased sound paint time must be a safe integer.
    #[error("rebased sound paint time must be a safe integer")]
    BadRebasedTime,
    /// listener entity must be a signed 32-bit integer.
    #[error("listener entity must be a signed 32-bit integer")]
    BadListener,
    /// effects volume has an undefined signed-int gain conversion.
    #[error("effects volume has an undefined signed-int gain conversion")]
    BadEffectsConversion,
    /// fixed-origin entity must be a signed 32-bit integer.
    #[error("fixed-origin entity must be a signed 32-bit integer")]
    BadFixedEntity,
    /// Sound start diagnostics require a registered name and console output.
    #[error("Sound start diagnostics require a registered name and console output")]
    StartDiagnostics,
    /// Sound playback ended during start diagnostics.
    #[error("Sound playback ended during start diagnostics")]
    PlaybackEnded,
    /// Source spatialization requires a voice policy.
    #[error("Source spatialization requires a voice policy")]
    MissingPolicy,
    /// Source effects require nonempty mono PCM.
    #[error("Source effects require nonempty mono PCM")]
    SourceEffectFormat,
    /// Invalid source sound gain or attenuation.
    #[error("Invalid source sound gain or attenuation")]
    BadSourceGain,
    /// Sound resamples to zero frames.
    #[error("Sound resamples to zero frames")]
    ZeroResample,
    /// Sound loop outside PCM.
    #[error("Sound loop outside PCM")]
    LoopOutsidePcm,
    /// Sound random source must return a nonnegative integer.
    #[error("Sound random source must return a nonnegative integer")]
    BadRandom,
    /// Static sound requires a WAV loop marker.
    #[error("Static sound requires a WAV loop marker")]
    StaticLoop,
    /// Ambient voice admission failed.
    #[error("Ambient voice admission failed")]
    AmbientAdmission,
    /// Missing sound cvar {0}.
    #[error("Missing sound cvar {0}")]
    MissingCvar(String),
    /// Invalid source sound timestamp.
    #[error("Invalid source sound timestamp")]
    BadSourceTimestamp,
    /// Sound deadline is outside the shared clock.
    #[error("Sound deadline is outside the shared clock")]
    BadDeadline,
    /// loop sound has length 0.
    #[error("loop sound has length 0")]
    ZeroLoop,
    /// paint start frame must be a safe integer.
    #[error("paint start frame must be a safe integer")]
    BadPaintStart,
    /// mix frame count must be a nonnegative safe integer.
    #[error("mix frame count must be a nonnegative safe integer")]
    BadMixFrames,
    /// mix output is too large.
    #[error("mix output is too large")]
    MixOutputTooLarge,
    /// paint end frame is outside the supported range.
    #[error("paint end frame is outside the supported range")]
    BadPaintEnd,
    /// direct consumption cannot rewind sound time.
    #[error("direct consumption cannot rewind sound time")]
    MixRewind,
    /// Channel scan left a pending sound.
    #[error("Channel scan left a pending sound")]
    PendingSound,
    /// resampled PCM frame count is outside the supported range.
    #[error("resampled PCM frame count is outside the supported range")]
    BadResampleFrames,
    /// sound allocation clock requires signed-int milliseconds.
    #[error("sound allocation clock requires signed-int milliseconds")]
    BadAllocationClock,
    /// Missing source sound position for entity {0}.
    #[error("Missing source sound position for entity {0}")]
    MissingEntityPosition(i64),
    /// loop sound reached an invalid negative sample access.
    #[error("loop sound reached an invalid negative sample access")]
    NegativeLoopAccess,
    /// Doppler range exceeds prepared samples.
    #[error("Doppler range exceeds prepared samples")]
    DopplerRange,
    /// Raw sample count and rate require source signed integers and a positive rate.
    #[error("Raw sample count and rate require source signed integers and a positive rate")]
    BadRawShape,
    /// raw volume has an undefined signed-int gain conversion.
    #[error("raw volume has an undefined signed-int gain conversion")]
    BadRawConversion,
    /// resampled raw PCM frame count is outside the supported range.
    #[error("resampled raw PCM frame count is outside the supported range")]
    BadRawFrames,
    /// raw PCM end frame is outside the supported range.
    #[error("raw PCM end frame is outside the supported range")]
    BadRawEnd,
    /// Raw sound diagnostics require console output.
    #[error("Raw sound diagnostics require console output")]
    RawDiagnostics,
    /// Missing initial WAV chunk.
    #[error("Missing initial WAV chunk")]
    MissingWavChunk,
    /// Invalid WAV {0} chunk length.
    #[error("Invalid WAV {0} chunk length")]
    BadWavChunk(String),
    /// WAV {0} chunk is outside the file.
    #[error("WAV {0} chunk is outside the file")]
    WavChunkOutside(String),
    /// WAV requires a format chunk.
    #[error("WAV requires a format chunk")]
    MissingWavFormat,
    /// WAV loop metadata is outside the file.
    #[error("WAV loop metadata is outside the file")]
    WavLoopOutside,
    /// Invalid WAV data offset.
    #[error("Invalid WAV data offset")]
    BadWavData,
    /// Invalid WAV sample width.
    #[error("Invalid WAV sample width")]
    BadWavWidth,
    /// Empty WAV data chunk.
    #[error("Empty WAV data chunk")]
    EmptyWav,
    /// Missing WAV data chunk.
    #[error("Missing WAV data chunk")]
    MissingWavData,
    /// Invalid WAV data length.
    #[error("Invalid WAV data length")]
    BadWavDataLength,
    /// WAV exceeds allocation capacity.
    #[error("WAV exceeds allocation capacity")]
    WavTooLarge,
    /// Zero-width WAV would divide by zero.
    #[error("Zero-width WAV would divide by zero")]
    ZeroWidthWav,
    /// PCM effect reads require mono samples.
    #[error("PCM effect reads require mono samples")]
    EffectReadStereo,
    /// Expected {want} PCM samples, decoded {have}.
    #[error("Expected {want} PCM samples, decoded {have}")]
    ShortDecode {
        /// Wanted.
        want: String,
        /// Have.
        have: String,
    },
    /// ADPCM predictor must be a signed 16-bit sample.
    #[error("ADPCM predictor must be a signed 16-bit sample")]
    BadAdpcmPredictor,
    /// ADPCM step index must be between 0 and 88.
    #[error("ADPCM step index must be between 0 and 88")]
    BadAdpcmIndex,
    /// ADPCM output is too short.
    #[error("ADPCM output is too short")]
    AdpcmOutputShort,
    /// ADPCM input is truncated.
    #[error("ADPCM input is truncated")]
    AdpcmInputTruncated,
    /// ADPCM chunk output needs 4096 samples.
    #[error("ADPCM chunk output needs 4096 samples")]
    AdpcmChunkOutput,
    /// S_AdpcmEncodeSound requires its initial sample.
    #[error("S_AdpcmEncodeSound requires its initial sample")]
    AdpcmMissingInitial,
    /// ADPCM soundData must be cleared before encoding.
    #[error("ADPCM soundData must be cleared before encoding")]
    AdpcmSoundDataSet,
    /// ADPCM memory inputs must be nonnegative signed 32-bit integers.
    #[error("ADPCM memory inputs must be nonnegative signed 32-bit integers")]
    AdpcmMemoryInputs,
    /// ADPCM sample rates must be positive.
    #[error("ADPCM sample rates must be positive")]
    AdpcmMemoryRates,
    /// ADPCM scaled sample count exceeds signed 32-bit range.
    #[error("ADPCM scaled sample count exceeds signed 32-bit range")]
    AdpcmMemoryScaled,
    /// Invalid ADPCM sound buffer length.
    #[error("Invalid ADPCM sound buffer length")]
    BadAdpcmLength,
    /// ADPCM sample would overflow the wave header range.
    #[error("ADPCM sample would overflow the wave header range")]
    AdpcmOverflow,
    /// Invalid ADPCM block alignment.
    #[error("Invalid ADPCM block alignment")]
    BadAdpcmBlock,
    /// Samples must be nonnegative safe integers.
    #[error("Samples must be nonnegative safe integers")]
    BadAdpcmSamples,
    /// {name} outside {minimum}..{maximum}.
    #[error("{name} outside {minimum}..{maximum}")]
    IntRange {
        /// Value name.
        name: String,
        /// Minimum.
        minimum: i64,
        /// Maximum.
        maximum: i64,
    },
    /// sound codec read outside allocation at {0}.
    #[error("sound codec read outside allocation at {0}")]
    WaveletRead(usize),
    /// odd daub4 size reads uninitialized source scratch.
    #[error("odd daub4 size reads uninitialized source scratch")]
    WaveletOdd,
    /// wavelet size below four does not terminate in source.
    #[error("wavelet size below four does not terminate in source")]
    WaveletSmall,
    /// sound codec destination is truncated.
    #[error("sound codec destination is truncated")]
    WaveletDest,
    /// NXPutc writes outside stream allocation.
    #[error("NXPutc writes outside stream allocation")]
    NxPutcBounds,
    /// source soundData must be null before compression.
    #[error("source soundData must be null before compression")]
    WaveletSoundData,
    /// Invalid wavelet packet length.
    #[error("Invalid wavelet packet length")]
    BadWaveletLength,
    /// Wavelet chunk must hold 1024 samples.
    #[error("Wavelet chunk must hold 1024 samples")]
    BadWaveletChunk,
    /// Invalid wavelet sample.
    #[error("Invalid wavelet sample")]
    BadWaveletSample,
    /// Sound chunk allocation failed.
    #[error("Sound chunk allocation failed")]
    ChunkAllocation,
    /// {name} must be a linear resampling of {have} Hz.
    #[error("{name} must be a linear resampling of {have} Hz")]
    BadResampleRate {
        /// Setting name.
        name: String,
        /// Rate.
        have: String,
    },
    /// Resampled sound has an undefined signed-int conversion.
    #[error("Resampled sound has an undefined signed-int conversion")]
    ResampleConversion,
    /// ResampleSfxRaw output is truncated.
    #[error("ResampleSfxRaw output is truncated")]
    ResampleTruncated,
    /// Sound paint access outside allocation at {0}.
    #[error("Sound paint access outside allocation at {0}")]
    PaintAccess(i64),
    /// Stereo blast requires an even sample count.
    #[error("Stereo blast requires an even sample count")]
    BlastCount,
    /// Stereo output allocation is truncated.
    #[error("Stereo output allocation is truncated")]
    BlastOutput,
    /// Invalid source DMA paint range or ring capacity.
    #[error("Invalid source DMA paint range or ring capacity")]
    DmaRange,
    /// Paint allocation is truncated.
    #[error("Paint allocation is truncated")]
    PaintTruncated,
    /// Sound paint reached a null source chunk.
    #[error("Sound paint reached a null source chunk")]
    NullPaintChunk,
    /// Invalid paint buffer length.
    #[error("Invalid paint buffer length")]
    BadPaintLength,
    /// Invalid DMA width.
    #[error("Invalid DMA width")]
    BadDmaWidth,
    /// Invalid paint output dimensions.
    #[error("Invalid paint output dimensions")]
    BadPaintDims,
    /// Invalid stereo paint chunk.
    #[error("Invalid stereo paint chunk")]
    BadStereoPaint,
    /// Painted reset requires enough stereo samples.
    #[error("Painted reset requires enough stereo samples")]
    ShortReset,
    /// Compressed paint requires a linked chunk.
    #[error("Compressed paint requires a linked chunk")]
    MissingPaintChunk,
    /// ADPCM paint reached an invalid chunk.
    #[error("ADPCM paint reached an invalid chunk")]
    BadPaintChunk,
    /// Paint scratch spans outside the sound.
    #[error("Paint scratch spans outside the sound")]
    PaintScratchSpan,
    /// Invalid paint count.
    #[error("Invalid paint count")]
    BadPaintCount,
    /// Invalid paint rate.
    #[error("Invalid paint rate")]
    BadPaintRate,
    /// PCM stream closed.
    #[error("PCM stream closed")]
    StreamClosed,
    /// Invalid PCM frame request.
    #[error("Invalid PCM frame request")]
    BadFrameRequest,
    /// PCM seek outside stream.
    #[error("PCM seek outside stream")]
    StreamSeek,
    /// Stream IO error: {0}.
    #[error("Stream IO error: {0}")]
    StreamIo(String),
    /// Invalid PCM checkpoint rate.
    #[error("Invalid PCM checkpoint rate")]
    BadCheckpointRate,
    /// Invalid PCM checkpoint field: {0}.
    #[error("Invalid PCM checkpoint field: {0}")]
    BadCheckpointField(String),
    /// sample exceeds signed PCM.
    #[error("sample exceeds signed PCM")]
    CheckpointSample,
    /// invalid PCM segment extent.
    #[error("invalid PCM segment extent")]
    CheckpointExtent,
    /// discontinuous PCM segments.
    #[error("discontinuous PCM segments")]
    CheckpointDiscontinuity,
    /// invalid PCM cursor.
    #[error("invalid PCM cursor")]
    CheckpointCursor,
    /// Invalid PCM output rate.
    #[error("Invalid PCM output rate")]
    BadPcmRate,
    /// Invalid streamed PCM.
    #[error("Invalid streamed PCM")]
    BadStreamedPcm,
    /// PCM stream format changed without reset.
    #[error("PCM stream format changed without reset")]
    StreamFormatChanged,
    /// PCM source discontinuity: expected {expected}, received {received}.
    #[error("PCM source discontinuity: expected {expected}, received {received}")]
    StreamDiscontinuity {
        /// Expected cursor.
        expected: i64,
        /// Received cursor.
        received: i64,
    },
    /// PCM source position outside queued segment.
    #[error("PCM source position outside queued segment")]
    StreamPosition,
    /// Invalid music volume.
    #[error("Invalid music volume")]
    BadMusicVolume,
    /// Invalid CD track.
    #[error("Invalid CD track")]
    BadCdTrack,
    /// CD remap exceeds 99 tracks.
    #[error("CD remap exceeds 99 tracks")]
    CdRemap,
    /// Expansion soundtrack index outside CD range.
    #[error("Expansion soundtrack index outside CD range")]
    XatrixTrack,
    /// Music control volume mode is invalid.
    #[error("Music control volume mode is invalid")]
    BadMusicMode,
    /// Music track {0} is outside 2..{1}.
    #[error("Music track {0} is outside 2..{1}")]
    BadMusicTrack(i64, i64),
    /// Music stream frames must stay within the track.
    #[error("Music stream frames must stay within the track")]
    MusicFrames,
    /// Music track seeks must stay within the track.
    #[error("Music track seeks must stay within the track")]
    MusicSeek,
    /// Music track reads must stay within the track.
    #[error("Music track reads must stay within the track")]
    MusicRead,
    /// Music track reads require a nonnegative position.
    #[error("Music track reads require a nonnegative position")]
    MusicPosition,
    /// Vorbis reads require {0}.
    #[error("Vorbis reads require {0}")]
    VorbisRead(String),
    /// Vorbis seeks require {0}.
    #[error("Vorbis seeks require {0}")]
    VorbisSeek(String),
    /// Unknown sound encoding: {0}.
    #[error("Unknown sound encoding: {0}")]
    UnknownEncoding(String),
    /// PCM checkpoint version mismatch.
    #[error("PCM checkpoint version mismatch")]
    CheckpointVersion,
    /// PCM checkpoint has no segments.
    #[error("PCM checkpoint has no segments")]
    CheckpointEmpty,
    /// PCM checkpoint offsets must increase.
    #[error("PCM checkpoint offsets must increase")]
    CheckpointOffsets,
    /// PCM checkpoint samples are outside {0} frames.
    #[error("PCM checkpoint samples are outside {0} frames")]
    CheckpointSamples(i64),
    /// PCM checkpoint never ends.
    #[error("PCM checkpoint never ends")]
    CheckpointEnd,
    /// PCM checkpoint stream ended.
    #[error("PCM checkpoint stream ended")]
    CheckpointStreamEnd,
    /// Expected enough PCM samples for checkpoint signal.
    #[error("Expected enough PCM samples for checkpoint signal")]
    CheckpointSignal,
    /// PCM checkpoint expects {0} frames, decoded {1}.
    #[error("PCM checkpoint expects {0} frames, decoded {1}")]
    CheckpointFrames(i64, i64),
    /// PCM checkpoint buffer is too large.
    #[error("PCM checkpoint buffer is too large")]
    CheckpointBuffer,
    /// Gain must be finite.
    #[error("Gain must be finite")]
    BadStreamGain,
    /// Invalid reverb sample rate.
    #[error("Invalid reverb sample rate")]
    BadReverbRate,
    /// Reverb requires stereo frames.
    #[error("Reverb requires stereo frames")]
    ReverbStereo,
    /// Audio time constant must be greater than zero.
    #[error("Audio time constant must be greater than zero")]
    BadTimeConstant,
    /// Audio impulse scale must be finite.
    #[error("Audio impulse scale must be finite")]
    BadImpulseScale,
    /// Sound environments require an environments array.
    #[error("Sound environments require an environments array")]
    EnvironmentsArray,
    /// Sound environment must be an object.
    #[error("Sound environment must be an object")]
    EnvironmentObject,
    /// Invalid sound environment dimension.
    #[error("Invalid sound environment dimension")]
    EnvironmentDimension,
    /// Sound environment reverbs must be an array.
    #[error("Sound environment reverbs must be an array")]
    EnvironmentReverbs,
    /// Sound reverb must be an object.
    #[error("Sound reverb must be an object")]
    ReverbObject,
    /// Sound reverb wildcard must begin with *.
    #[error("Sound reverb wildcard must begin with *")]
    ReverbWildcard,
    /// Sound reverb materials must be an array or wildcard.
    #[error("Sound reverb materials must be an array or wildcard")]
    ReverbMaterials,
    /// Sound material must be text.
    #[error("Sound material must be text")]
    MaterialText,
    /// Sound reverb preset must be text.
    #[error("Sound reverb preset must be text")]
    ReverbPresetText,
    /// Unknown reverb preset {0}.
    #[error("Unknown reverb preset {0}")]
    UnknownPreset(usize),
    /// Reverb time must be finite.
    #[error("Reverb time must be finite")]
    ReverbTime,
    /// Reverb probe index outside fixed directions.
    #[error("Reverb probe index outside fixed directions")]
    ReverbProbe,
    /// Reverb environment disappeared.
    #[error("Reverb environment disappeared")]
    ReverbEnvironmentGone,
    /// Sound environments require an environments array.
    #[error("Sound environments require an environments array")]
    EnvironmentsShape,
    /// Sound environments require an object.
    #[error("Sound environments require an object")]
    EnvironmentShape,
    /// Sound environment volumes require at least three vertices.
    #[error("Sound environment volumes require at least three vertices")]
    EnvironmentVertices,
    /// Sound environment corners must be numbers.
    #[error("Sound environment corners must be numbers")]
    EnvironmentCorners,
    /// Sound environments must be valid JSON: {0}.
    #[error("Sound environments must be valid JSON: {0}")]
    EnvironmentsJson(String),
    /// Reverb preset {0} is outside 0..{1}.
    #[error("Reverb preset {0} is outside 0..{1}")]
    BadPreset(i64, usize),
    /// Reverb delay line reads require a nonnegative delay.
    #[error("Reverb delay line reads require a nonnegative delay")]
    NegativeDelay,
    /// Reverb delay line reads require {0}.
    #[error("Reverb delay line reads require {0}")]
    BadDelay(String),
    /// Reverb networks require positive delays.
    #[error("Reverb networks require positive delays")]
    BadNetworkDelays,
    /// Missing registered sound: {0}.
    #[error("Missing registered sound: {0}")]
    MissingSound(String),
    /// Sound name mismatch: {0}.
    #[error("Sound name mismatch: {0}")]
    SoundName(String),
    /// Missing registered music: {0}.
    #[error("Missing registered music: {0}")]
    MissingMusic(String),
    /// Invalid music extension: {0}.
    #[error("Invalid music extension: {0}")]
    BadMusicExtension(String),
    /// Missing music track: {0}.
    #[error("Missing music track: {0}")]
    MissingMusicTrack(String),
    /// Bank content error: {0}.
    #[error("Bank content error: {0}")]
    BankContent(String),
    /// Cvar error: {0}.
    #[error("Cvar error: {0}")]
    Cvar(String),
    /// Binary decode error: {0}.
    #[error("Binary decode error: {0}")]
    Binary(String),
    /// Platform audio error: {0}.
    #[error("Platform audio error: {0}")]
    Platform(String),
}

impl From<qa_core::cvar::CvarError> for AudioError {
    fn from(error: qa_core::cvar::CvarError) -> Self {
        AudioError::Cvar(error.to_string())
    }
}

impl From<qa_core::binary::BinaryError> for AudioError {
    fn from(error: qa_core::binary::BinaryError) -> Self {
        AudioError::Binary(error.to_string())
    }
}

impl From<qa_platform::error::Error> for AudioError {
    fn from(error: qa_platform::error::Error) -> Self {
        AudioError::Platform(error.to_string())
    }
}

/// Whether a platform error is an unavailable-device error.
#[must_use]
pub fn is_unavailable(error: &qa_platform::error::Error) -> bool {
    matches!(error, qa_platform::error::Error::Unavailable { .. })
}
