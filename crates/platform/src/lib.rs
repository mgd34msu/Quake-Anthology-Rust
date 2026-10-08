mod sdl;

#[cfg(any(debug_assertions, feature = "allocation-tracking"))]
pub mod allocations;

pub use sdl::{InputEvent, Window};
