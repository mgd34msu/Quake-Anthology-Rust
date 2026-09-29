//! Shared dynamic-loading plumbing for the native bindings.
//!
//! Each binding keeps its [`LoadedLibrary`] alive for as long as any resolved
//! function pointer may be called. Symbols resolve eagerly at load time so a
//! library missing one required entry point still reports
//! [`Error::Unavailable`] naming the library, never a half-loaded handle.

use std::ffi::c_void;
use std::mem;

use libloading::Library;

use crate::error::{Error, Result};
use crate::native_libraries::{open_native_library, NativeLibrary, NativeLibraryOptions};

/// A successfully opened native library.
pub struct LoadedLibrary {
    kind: NativeLibrary,
    library: Library,
}

impl LoadedLibrary {
    /// Open `kind` through ordered candidate discovery.
    ///
    /// # Safety
    ///
    /// Loading a native library is safe in itself; calling resolved symbols
    /// requires each binding's documented preconditions. The `unsafe` marker
    /// forces callers to acknowledge foreign code execution.
    pub unsafe fn open(kind: NativeLibrary, options: &NativeLibraryOptions) -> Result<Self> {
        // SAFETY: `Library::new` only maps the image; no symbols are called.
        let library = open_native_library(
            kind,
            |path| {
                // SAFETY: opening maps the image without invoking its code.
                unsafe { Library::new(path) }.map_err(|error| Error::native("dlopen", error.to_string()))
            },
            options,
        )?;
        Ok(Self { kind, library })
    }

    /// Resolve a required symbol, or [`Error::Unavailable`] naming the library.
    ///
    /// # Safety
    ///
    /// The caller must name the exact C signature in `T`; the returned pointer
    /// is valid while `self` is alive.
    pub unsafe fn symbol<T>(&self, name: &[u8]) -> Result<T>
    where
        T: Copy,
    {
        // SAFETY: the raw address is only read; the caller guarantees the type.
        unsafe {
            self.library
                .get::<*mut c_void>(name)
                .map(|symbol| mem::transmute_copy::<*mut c_void, T>(&symbol))
                .map_err(|_| {
                    Error::unavailable(
                        self.kind.name(),
                        format!(
                            "symbol `{}` is missing",
                            String::from_utf8_lossy(name).trim_end_matches('\0')
                        ),
                    )
                })
        }
    }

    /// Library family, for error attribution.
    #[must_use]
    pub fn kind(&self) -> NativeLibrary {
        self.kind
    }
}

/// Read a NUL-terminated C string without decoding or copying more than needed.
///
/// # Safety
///
/// `ptr` must be a valid NUL-terminated byte string owned by the native
/// library for the duration of the call.
pub unsafe fn c_string_bytes(ptr: *const u8) -> Vec<u8> {
    // SAFETY: caller guarantees a valid NUL-terminated string.
    unsafe {
        let mut length = 0;
        while *ptr.add(length) != 0 {
            length += 1;
        }
        std::slice::from_raw_parts(ptr, length).to_vec()
    }
}

/// Read a NUL-terminated C string as lossy UTF-8.
///
/// # Safety
///
/// `ptr` must be a valid NUL-terminated byte string owned by the native
/// library for the duration of the call.
pub unsafe fn c_string_lossy(ptr: *const u8) -> String {
    // SAFETY: caller guarantees a valid NUL-terminated string.
    unsafe { String::from_utf8_lossy(&c_string_bytes(ptr)).into_owned() }
}

/// Encode `value` as a NUL-terminated UTF-8 buffer, rejecting embedded NULs.
pub fn c_string(value: &str) -> Result<Vec<u8>> {
    if value.contains('\0') {
        return Err(Error::InvalidInput("native string contains NUL".to_string()));
    }
    let mut bytes = Vec::with_capacity(value.len() + 1);
    bytes.extend_from_slice(value.as_bytes());
    bytes.push(0);
    Ok(bytes)
}

/// Last-error text for SDL-style `GetError` callbacks.
///
/// # Safety
///
/// `get_error` must return a valid NUL-terminated string.
pub unsafe fn sdl_error(get_error: unsafe extern "C" fn() -> *const u8) -> String {
    // SAFETY: SDL guarantees a valid thread-local error string.
    unsafe { c_string_lossy(get_error()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_library_names_itself() {
        let mut environment = std::collections::HashMap::new();
        environment.insert(
            "QUAKE_SDL2_LIBRARY".to_string(),
            "/nonexistent-qa-platform/libSDL2.so".to_string(),
        );
        let options = NativeLibraryOptions {
            environment: Some(environment),
            ..NativeLibraryOptions::default()
        };
        // SAFETY: opening never invokes foreign code.
        let Err(error) = (unsafe { LoadedLibrary::open(NativeLibrary::Sdl2, &options) }) else {
            panic!("expected failure")
        };
        assert!(error.is_unavailable(), "{error}");
        assert!(error.to_string().contains("sdl2"), "{error}");
        assert!(
            error.to_string().contains("/nonexistent-qa-platform/libSDL2.so"),
            "{error}"
        );
    }

    #[test]
    fn missing_symbol_names_library() {
        // libc is always present on Linux but has no SDL entry points, which
        // exercises the missing-symbol fallback without system SDL installed.
        if !cfg!(target_os = "linux") {
            return;
        }
        let mut environment = std::collections::HashMap::new();
        environment.insert("QUAKE_SDL2_LIBRARY".to_string(), "libc.so.6".to_string());
        let options = NativeLibraryOptions {
            environment: Some(environment),
            ..NativeLibraryOptions::default()
        };
        // SAFETY: opening never invokes foreign code.
        let library = unsafe { LoadedLibrary::open(NativeLibrary::Sdl2, &options) }.unwrap();
        // SAFETY: resolution only reads the symbol address.
        let Err(error) = (unsafe { library.symbol::<unsafe extern "C" fn()>(b"SDL_CreateWindow\0") }) else {
            panic!("expected failure")
        };
        assert!(error.is_unavailable(), "{error}");
        assert!(error.to_string().contains("sdl2"), "{error}");
        assert!(error.to_string().contains("SDL_CreateWindow"), "{error}");
    }

    #[test]
    fn c_string_rejects_nul() {
        assert_eq!(c_string("ok").unwrap(), b"ok\0");
        assert!(c_string("bad\0").is_err());
    }
}
