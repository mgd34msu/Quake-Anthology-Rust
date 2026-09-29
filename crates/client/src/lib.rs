//! Headless client core: prediction, view, HUD, input, audio, rendering,
//! spline cameras, and frame capture.
//!
//! Ported from the TypeScript donor's client-side presentation: movement
//! prediction (`src/network/q1/prediction.ts`,
//! `src/network/q2/prediction.ts`, `src/movement/q3/prediction.ts`,
//! `src/content/q3/presentation/prediction.ts`), view computation
//! (`src/content/q3/presentation/view.ts`, `src/render/scene/view.ts`),
//! HUD state (`src/ui/hud/*`, `src/content/q3/presentation/draw-tools.ts`),
//! input (`src/input/*`), audio hooks (`src/audio/mixer.ts`,
//! `src/audio/types.ts`), scene submission
//! (`src/contracts/{render,scene}.ts`, `src/render/scene/*`), spline
//! cameras (`src/camera/{application,spline}.ts`), and frame capture
//! (`src/capture/index.ts`).
//!
//! Platform backends (windowing, graphics, real audio) stay out: this
//! crate exposes traits plus headless implementations only. Image
//! encoders for capture stay out with the deferred `qa-content` image
//! formats; capture takes them as injected callbacks.

pub mod audio;
pub mod camera;
pub mod capture;
pub mod hud;
pub mod input;
pub mod materials;
pub mod media;
pub mod prediction;
pub mod render;
pub mod text;
pub mod ui;
pub mod view;

mod error;

pub use error::ClientError;
