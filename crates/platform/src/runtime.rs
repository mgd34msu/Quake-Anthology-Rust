//! Platform runtime: dedicated or graphical startup.
//!
//! Port of donor `src/platform/runtime.ts`. Importing the platform opens no
//! library; dedicated startup never initializes SDL.

use crate::error::Result;
use crate::native_libraries::NativeLibraryOptions;
use crate::sdl::{SdlWindow, SdlWindowOptions};

/// Platform startup options.
#[derive(Clone, Debug)]
pub enum PlatformOptions {
    /// Headless dedicated server: no window, no SDL.
    Dedicated,
    /// Graphical client with an SDL window.
    Graphical {
        /// Window options.
        window: SdlWindowOptions,
    },
}

/// Dedicated platform handle.
pub struct DedicatedPlatform {
    stopped: bool,
}

impl DedicatedPlatform {
    /// Whether the platform is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.stopped
    }

    /// Stop the platform. Idempotent.
    pub fn close(&mut self) {
        self.stopped = true;
    }
}

impl Drop for DedicatedPlatform {
    fn drop(&mut self) {
        self.close();
    }
}

/// Graphical platform handle owning its window.
pub struct GraphicalPlatform {
    window: SdlWindow,
}

impl GraphicalPlatform {
    /// Borrow the window.
    #[must_use]
    pub fn window(&self) -> &SdlWindow {
        &self.window
    }

    /// Mutably borrow the window.
    pub fn window_mut(&mut self) -> &mut SdlWindow {
        &mut self.window
    }

    /// Whether the window is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.window.is_closed()
    }

    /// Close the window. Idempotent once closed.
    pub fn close(&mut self) -> Result<()> {
        self.window.close()
    }
}

/// An open platform.
pub enum Platform {
    /// Dedicated server platform.
    Dedicated(DedicatedPlatform),
    /// Graphical client platform (boxed: the window dwarfs the dedicated handle).
    Graphical(Box<GraphicalPlatform>),
}

impl Platform {
    /// Whether the platform is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        match self {
            Self::Dedicated(platform) => platform.is_closed(),
            Self::Graphical(platform) => platform.is_closed(),
        }
    }
}

/// Open the platform with live process discovery.
pub fn open_platform(options: &PlatformOptions) -> Result<Platform> {
    open_platform_with(options, &NativeLibraryOptions::default())
}

/// Open with explicit library discovery (tests inject overrides).
pub fn open_platform_with(options: &PlatformOptions, lib_options: &NativeLibraryOptions) -> Result<Platform> {
    match options {
        PlatformOptions::Dedicated => Ok(Platform::Dedicated(DedicatedPlatform { stopped: false })),
        PlatformOptions::Graphical { window } => {
            let window = SdlWindow::open_with(window, lib_options)?;
            Ok(Platform::Graphical(Box::new(GraphicalPlatform { window })))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdl::SdlWindowOptions;

    fn missing_lib() -> NativeLibraryOptions {
        let mut environment = std::collections::HashMap::new();
        environment.insert(
            "QUAKE_SDL2_LIBRARY".to_string(),
            "/nonexistent-qa-platform/libSDL2.so".to_string(),
        );
        NativeLibraryOptions {
            environment: Some(environment),
            ..NativeLibraryOptions::default()
        }
    }

    #[test]
    fn dedicated_needs_no_library() {
        // Dedicated startup never initializes SDL, so it opens without libs.
        let mut platform = open_platform_with(&PlatformOptions::Dedicated, &missing_lib()).unwrap();
        assert!(!platform.is_closed());
        match &mut platform {
            Platform::Dedicated(dedicated) => {
                dedicated.close();
                assert!(dedicated.is_closed());
                dedicated.close();
            }
            Platform::Graphical(_) => panic!("expected dedicated"),
        }
        assert!(platform.is_closed());
    }

    #[test]
    fn graphical_without_library_names_sdl2() {
        let window = SdlWindowOptions {
            title: "qa".to_string(),
            width: 64,
            height: 64,
            ..SdlWindowOptions::default()
        };
        let Err(error) = open_platform_with(&PlatformOptions::Graphical { window }, &missing_lib()) else {
            panic!("expected failure")
        };
        assert!(error.is_unavailable(), "{error}");
        assert!(error.to_string().contains("sdl2"), "{error}");
    }

    #[test]
    fn live_graphical_reports_honestly() {
        let window = SdlWindowOptions {
            title: "qa-platform probe".to_string(),
            width: 64,
            height: 64,
            hidden: true,
            ..SdlWindowOptions::default()
        };
        match open_platform(&PlatformOptions::Graphical { window }) {
            Ok(Platform::Graphical(mut platform)) => {
                assert!(!platform.is_closed());
                platform.close().unwrap();
                assert!(platform.is_closed());
            }
            Ok(Platform::Dedicated(_)) => panic!("expected graphical"),
            Err(error) => assert!(!error.to_string().is_empty(), "{error}"),
        }
    }
}
