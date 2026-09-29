//! Media: cinematic containers, codecs, timelines, and presentation.
//!
//! Ported from the TypeScript donor's `src/media/*`: stream clocks and
//! timelines (`playback`), container readers (`source`, `cin`,
//! `roq-stream`, `roq`, `ogg` in `containers`), codec engines
//! (`roq-codebook`, `roq-audio`, `theora`, `vorbis`), still frames
//! (`still`), transitions (`transitions`), audio routing (`audio`),
//! material and fullscreen presentation (`material`, `presentation`,
//! `roq-presentation`), and shared types (`types`).
//!
//! Decoders produce real RGBA frames and PCM audio. No GPU calls are
//! made from these modules, keeping `NullRenderer` compatibility.

pub mod audio;
pub mod cin;
pub mod cin_playback;
pub mod containers;
pub mod material;
pub mod ogv_playback;
pub mod playback;
pub mod presentation;
pub mod roq;
pub mod roq_audio;
pub mod roq_codebook;
pub mod roq_playback;
pub mod roq_presentation;
pub mod roq_stream;
pub mod source;
pub mod still;
pub mod theora;
pub mod transitions;
pub mod types;
pub mod vorbis;
