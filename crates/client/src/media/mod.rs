//! Media: cinematic containers, timelines, and presentation.
//!
//! Ported from the TypeScript donor's `src/media/*`: stream clocks and
//! timelines (`playback`), container readers (`source`, `cin`,
//! `roq-stream`, `roq`, `ogg`), still frames (`still`), transitions
//! (`transitions`), audio routing (`audio`), material and fullscreen
//! presentation (`material`, `presentation`, `roq-presentation`), and
//! shared types (`types`).
//!
//! Timing-only container frames: CIN/RoQ/OGV readers parse headers and
//! chunk structure with real dimensions, indices, and times but leave
//! pixel and audio decode to deferred engines (`DecoderStatus`).
//! Stills decode synchronously. No GPU calls are made from these
//! modules, keeping `NullRenderer` compatibility.

pub mod audio;
pub mod containers;
pub mod material;
pub mod playback;
pub mod presentation;
pub mod source;
pub mod still;
pub mod transitions;
pub mod types;
