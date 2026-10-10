mod audio;
mod clock;
mod cpu;
mod events;
pub mod native;
mod profile;
mod sdl;
mod stdin;
mod workers;

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
pub mod allocations;

pub use audio::AudioStream;
pub use clock::{Stopwatch, pause};
pub use cpu::physical_core_count;
pub use events::EventPump;
pub use profile::saved_profile_root;
pub use sdl::Window;
pub use workers::{MAX_WORKERS, WorkerError, Workers};
