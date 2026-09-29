//! Platform natives: windows, audio, controllers, fonts, codecs, and sockets.
//!
//! Port of donor `src/platform/*`. System libraries (SDL, GL, FreeType,
//! Vorbis, Theora) load dynamically through
//! [`native_libraries`](crate::native_libraries) discovery; every absence
//! reports [`Error::Unavailable`] naming the library. File storage and IPX
//! sockets build on `std` plus always-present system symbols.

pub mod audio;
pub mod controller;
pub mod error;
pub mod ffi_util;
pub mod files;
pub mod freetype;
pub mod freetype_layout;
pub mod gl;
pub mod gl_framebuffers;
pub mod gl_programs;
pub mod ipx;
pub mod ipx_native;
pub mod native_libraries;
pub mod runtime;
pub mod sdl;
pub mod sdl_render_context;
pub mod theora;
pub mod vorbis;

pub use error::{AggregateList, Error, Result};
