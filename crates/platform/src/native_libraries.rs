//! Typed discovery of the approved native library families.
//!
//! Port of donor `src/platform/native-libraries.ts`: ordered loader inputs,
//! never evidence that a library exists. An explicit `QUAKE_*_LIBRARY`
//! override is exclusive; otherwise candidates pair each well-known file name
//! with the executable directory first and the bare name (dynamic loader
//! search path) last.

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Approved native library families.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NativeLibrary {
    /// SDL 2.x (`libSDL2-2.0.so.0`, `SDL2.dll`, ...).
    Sdl2,
    /// SDL 3.x audio (`libSDL3.so.0`, `SDL3.dll`, ...).
    Sdl3,
    /// System OpenGL (`libGL.so.1`, `opengl32.dll`, ...).
    Gl,
    /// Ogg Vorbis decode (`libvorbisfile.so.3`, ...).
    VorbisFile,
    /// Theora decode (`libtheoradec.so.2`, ...).
    TheoraDec,
    /// FreeType (`libfreetype.so.6`, ...).
    FreeType,
}

impl NativeLibrary {
    /// Donor short name; also used in [`Error::Unavailable`].
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Sdl2 => "sdl2",
            Self::Sdl3 => "sdl3",
            Self::Gl => "gl",
            Self::VorbisFile => "vorbisfile",
            Self::TheoraDec => "theoradec",
            Self::FreeType => "freetype",
        }
    }

    pub(crate) fn env_override(self) -> &'static str {
        match self {
            Self::Sdl2 => "QUAKE_SDL2_LIBRARY",
            Self::Sdl3 => "QUAKE_SDL3_LIBRARY",
            Self::Gl => "QUAKE_GL_LIBRARY",
            Self::VorbisFile => "QUAKE_VORBISFILE_LIBRARY",
            Self::TheoraDec => "QUAKE_THEORA_LIBRARY",
            Self::FreeType => "QUAKE_FREETYPE_LIBRARY",
        }
    }

    fn linux_names(self) -> &'static [&'static str] {
        match self {
            Self::Sdl2 => &["libSDL2-2.0.so.0", "libSDL2.so"],
            Self::Sdl3 => &["libSDL3.so.0", "libSDL3.so"],
            Self::Gl => &["libGL.so.1", "libGL.so"],
            Self::VorbisFile => &["libvorbisfile.so.3", "libvorbisfile.so"],
            Self::TheoraDec => &["libtheoradec.so.2", "libtheoradec.so"],
            Self::FreeType => &["libfreetype.so.6", "libfreetype.so"],
        }
    }

    fn windows_names(self) -> &'static [&'static str] {
        match self {
            Self::Sdl2 => &["SDL2.dll"],
            Self::Sdl3 => &["SDL3.dll"],
            Self::Gl => &["opengl32.dll"],
            Self::VorbisFile => &["libvorbisfile-3.dll", "vorbisfile.dll"],
            Self::TheoraDec => &["libtheoradec-1.dll", "theoradec.dll"],
            Self::FreeType => &["freetype.dll", "libfreetype-6.dll", "freetype6.dll"],
        }
    }

    fn macos_names(self) -> Vec<String> {
        match self {
            Self::Sdl2 => vec!["libSDL2-2.0.0.dylib".into(), "libSDL2.dylib".into()],
            Self::Sdl3 => vec!["libSDL3.0.dylib".into(), "libSDL3.dylib".into()],
            Self::Gl => vec![default_opengl_driver("darwin").unwrap_or("libGL.dylib").to_string()],
            Self::VorbisFile => vec!["libvorbisfile.3.dylib".into(), "libvorbisfile.dylib".into()],
            Self::TheoraDec => vec!["libtheoradec.2.dylib".into(), "libtheoradec.dylib".into()],
            Self::FreeType => vec!["libfreetype.6.dylib".into(), "libfreetype.dylib".into()],
        }
    }
}

/// Inputs for candidate discovery. Every field defaults to the live process
/// value; tests inject synthetic values for repo-layout coverage.
#[derive(Clone, Debug, Default)]
pub struct NativeLibraryOptions {
    /// `linux` / `win32` / `darwin`. Defaults to the host platform.
    pub platform: Option<String>,
    /// Executable path whose directory anchors packaged candidates.
    /// Defaults to the current executable.
    pub exec_path: Option<PathBuf>,
    /// Home directory for macOS framework lookup. Defaults to `$HOME`.
    pub home_directory: Option<PathBuf>,
    /// Environment map. Defaults to the live process environment.
    pub environment: Option<HashMap<String, String>>,
}

impl NativeLibraryOptions {
    fn platform_name(&self) -> String {
        if let Some(platform) = &self.platform {
            return platform.clone();
        }
        if cfg!(windows) {
            "win32".to_string()
        } else if cfg!(target_os = "macos") {
            "darwin".to_string()
        } else {
            "linux".to_string()
        }
    }

    fn exec_dir(&self) -> PathBuf {
        if let Some(path) = &self.exec_path {
            if let Some(parent) = Path::new(path).parent() {
                if !parent.as_os_str().is_empty() {
                    return parent.to_path_buf();
                }
            }
            return PathBuf::new();
        }
        env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .unwrap_or_default()
    }

    fn home_dir(&self) -> PathBuf {
        if let Some(home) = &self.home_directory {
            return home.clone();
        }
        env::var_os("HOME").map_or_else(PathBuf::new, PathBuf::from)
    }

    fn lookup_env(&self, name: &str) -> Option<String> {
        if let Some(map) = &self.environment {
            return map.get(name).cloned();
        }
        env::var(name).ok()
    }

    /// Whether an explicit `QUAKE_*_LIBRARY` override is set for `kind`.
    pub fn is_override_set(&self, kind: NativeLibrary) -> bool {
        self.lookup_env(kind.env_override()).is_some()
    }
}

/// Ordered loader inputs for `kind`, mirroring donor ordering exactly:
/// executable-directory joins, then platform install locations, then bare names.
pub fn native_library_candidates(kind: NativeLibrary, options: &NativeLibraryOptions) -> Result<Vec<String>> {
    let variable = kind.env_override();
    if let Some(path) = options.lookup_env(variable) {
        if path.is_empty() || path.contains('\0') {
            return Err(Error::InvalidInput(format!(
                "{variable} must be nonempty and contain no NUL"
            )));
        }
        return Ok(vec![path]);
    }
    let platform = options.platform_name();
    let exec_dir = options.exec_dir();
    let join_exec = |name: &str| -> String {
        if exec_dir.as_os_str().is_empty() {
            name.to_string()
        } else {
            exec_dir.join(name).to_string_lossy().into_owned()
        }
    };
    match platform.as_str() {
        "linux" => {
            let names = kind.linux_names();
            let mut out = Vec::with_capacity(names.len() * 2);
            out.extend(names.iter().map(|name| join_exec(name)));
            out.extend(names.iter().map(ToString::to_string));
            Ok(out)
        }
        "win32" => {
            let names = kind.windows_names();
            let mut out = Vec::with_capacity(names.len() * 2);
            out.extend(names.iter().map(|name| join_exec(name)));
            out.extend(names.iter().map(ToString::to_string));
            Ok(out)
        }
        "darwin" => {
            let names = kind.macos_names();
            let mut installed = Vec::new();
            for dir in ["/opt/homebrew/lib", "/usr/local/lib"] {
                for name in &names {
                    installed.push(format!("{dir}/{name}"));
                }
            }
            if matches!(kind, NativeLibrary::Sdl2 | NativeLibrary::Sdl3) {
                let framework = if kind == NativeLibrary::Sdl3 {
                    "SDL3.framework/SDL3"
                } else {
                    "SDL2.framework/SDL2"
                };
                let home_frameworks = options.home_dir().join("Library/Frameworks");
                for dir in [
                    exec_dir.to_string_lossy().into_owned(),
                    home_frameworks.to_string_lossy().into_owned(),
                    "/Library/Frameworks".to_string(),
                ] {
                    installed.push(format!("{dir}/{framework}"));
                }
            }
            let mut out = Vec::with_capacity(names.len() * 2 + installed.len());
            out.extend(names.iter().map(|name| join_exec(name)));
            out.extend(installed);
            out.extend(names);
            Ok(out)
        }
        other => Err(Error::Unsupported(format!(
            "native {} libraries are unsupported on {other}",
            kind.name()
        ))),
    }
}

/// Try each candidate in order with `open`, returning the first success.
/// Failure aggregates every attempt into [`Error::Unavailable`] naming `kind`.
pub fn open_native_library<T>(
    kind: NativeLibrary,
    mut open: impl FnMut(&str) -> Result<T>,
    options: &NativeLibraryOptions,
) -> Result<T> {
    let candidates = native_library_candidates(kind, options)?;
    let mut failures = Vec::new();
    for candidate in &candidates {
        match open(candidate) {
            Ok(value) => return Ok(value),
            Err(error) => failures.push(format!("{candidate}: {error}")),
        }
    }
    Err(Error::unavailable(
        kind.name(),
        format!("could not load {}; attempted {}", kind.name(), candidates.join(", ")),
    )
    .with_attempts(failures))
}

/// Concrete system OpenGL driver path for `platform`.
pub fn default_opengl_driver(platform: &str) -> Result<&'static str> {
    match platform {
        "linux" => Ok("libGL.so.1"),
        "darwin" => Ok("/System/Library/Frameworks/OpenGL.framework/Libraries/libGL.dylib"),
        "win32" => Ok("opengl32.dll"),
        other => Err(Error::Unsupported(format!("OpenGL is unsupported on {other}"))),
    }
}

/// Host default driver path.
pub fn host_opengl_driver() -> Result<&'static str> {
    if cfg!(windows) {
        default_opengl_driver("win32")
    } else if cfg!(target_os = "macos") {
        default_opengl_driver("darwin")
    } else {
        default_opengl_driver("linux")
    }
}

trait WithAttempts {
    fn with_attempts(self, failures: Vec<String>) -> Self;
}

impl WithAttempts for Error {
    fn with_attempts(self, failures: Vec<String>) -> Self {
        match self {
            Self::Unavailable { library, detail } => {
                let mut full = detail;
                for failure in failures {
                    full.push_str("\n  ");
                    full.push_str(&failure);
                }
                Self::Unavailable { library, detail: full }
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(platform: &str, exec: &str) -> NativeLibraryOptions {
        NativeLibraryOptions {
            platform: Some(platform.to_string()),
            exec_path: Some(PathBuf::from(exec)),
            environment: Some(HashMap::new()),
            ..NativeLibraryOptions::default()
        }
    }

    #[test]
    fn linux_discovery_retains_approved_families() {
        let options = opts("linux", "/opt/quake/quake");
        assert_eq!(
            native_library_candidates(NativeLibrary::Sdl2, &options).unwrap(),
            vec![
                "/opt/quake/libSDL2-2.0.so.0",
                "/opt/quake/libSDL2.so",
                "libSDL2-2.0.so.0",
                "libSDL2.so",
            ]
        );
        assert_eq!(
            native_library_candidates(NativeLibrary::Sdl3, &options).unwrap(),
            vec![
                "/opt/quake/libSDL3.so.0",
                "/opt/quake/libSDL3.so",
                "libSDL3.so.0",
                "libSDL3.so",
            ]
        );
        assert!(native_library_candidates(NativeLibrary::Gl, &options)
            .unwrap()
            .contains(&"libGL.so.1".to_string()));
        assert!(native_library_candidates(NativeLibrary::VorbisFile, &options)
            .unwrap()
            .contains(&"libvorbisfile.so.3".to_string()));
        assert!(native_library_candidates(NativeLibrary::FreeType, &options)
            .unwrap()
            .contains(&"libfreetype.so.6".to_string()));
    }

    #[test]
    fn explicit_override_is_exclusive_and_reports_failing_path() {
        let mut env = HashMap::new();
        env.insert("QUAKE_SDL2_LIBRARY".to_string(), "/missing/libSDL2.so".to_string());
        let options = NativeLibraryOptions {
            platform: Some("linux".to_string()),
            environment: Some(env),
            ..NativeLibraryOptions::default()
        };
        assert_eq!(
            native_library_candidates(NativeLibrary::Sdl2, &options).unwrap(),
            vec!["/missing/libSDL2.so".to_string()]
        );
        let mut attempted = Vec::new();
        let Err(error) = open_native_library(
            NativeLibrary::Sdl2,
            |path: &str| -> Result<()> {
                attempted.push(path.to_string());
                Err(Error::native("dlopen", "native loader rejected this candidate"))
            },
            &options,
        ) else {
            panic!("expected failure")
        };
        assert!(error.to_string().contains("/missing/libSDL2.so"), "{error}");
        assert!(error.to_string().contains("sdl2"), "{error}");
        assert!(error.is_unavailable());
        assert_eq!(attempted, vec!["/missing/libSDL2.so".to_string()]);
        for value in ["", "bad\0path"] {
            let mut env = HashMap::new();
            env.insert("QUAKE_SDL2_LIBRARY".to_string(), value.to_string());
            let bad = NativeLibraryOptions {
                environment: Some(env),
                ..NativeLibraryOptions::default()
            };
            assert!(native_library_candidates(NativeLibrary::Sdl2, &bad)
                .unwrap_err()
                .to_string()
                .contains("nonempty"));
        }
    }

    #[test]
    fn discovery_tries_candidates_in_order() {
        let options = opts("linux", "/missing/quake");
        // Point the packaged names at a missing dir but let the bare loader
        // name succeed, mirroring the donor's ordered-attempt test.
        let mut attempted = Vec::new();
        let loaded = open_native_library(
            NativeLibrary::VorbisFile,
            |path: &str| -> Result<String> {
                attempted.push(path.to_string());
                if path.starts_with("/missing/") {
                    return Err(Error::native("dlopen", "missing packaged library"));
                }
                Ok(path.to_string())
            },
            &options,
        )
        .unwrap();
        assert_eq!(loaded, "libvorbisfile.so.3");
        assert_eq!(
            attempted,
            vec![
                "/missing/libvorbisfile.so.3".to_string(),
                "/missing/libvorbisfile.so".to_string(),
                "libvorbisfile.so.3".to_string(),
            ]
        );
    }

    #[test]
    fn sdl3_identities_per_platform() {
        let mut env = HashMap::new();
        env.insert("QUAKE_SDL3_LIBRARY".to_string(), "/private/SDL3".to_string());
        let options = NativeLibraryOptions {
            environment: Some(env),
            ..NativeLibraryOptions::default()
        };
        assert_eq!(
            native_library_candidates(NativeLibrary::Sdl3, &options).unwrap(),
            vec!["/private/SDL3".to_string()]
        );
        let win = NativeLibraryOptions {
            platform: Some("win32".to_string()),
            exec_path: Some(PathBuf::from("quake.exe")),
            environment: Some(HashMap::new()),
            ..NativeLibraryOptions::default()
        };
        assert!(native_library_candidates(NativeLibrary::Sdl3, &win)
            .unwrap()
            .iter()
            .any(|c| c.contains("SDL3.dll")));
        let mac = NativeLibraryOptions {
            platform: Some("darwin".to_string()),
            exec_path: Some(PathBuf::from("/app/quake")),
            home_directory: Some(PathBuf::from("/private/home")),
            environment: Some(HashMap::new()),
        };
        assert!(native_library_candidates(NativeLibrary::Sdl3, &mac)
            .unwrap()
            .contains(&"/Library/Frameworks/SDL3.framework/SDL3".to_string()));
    }

    #[test]
    fn unsupported_platform_names_library() {
        let options = opts("freebsd", "/bin/quake");
        let Err(error) = native_library_candidates(NativeLibrary::Gl, &options) else {
            panic!("expected failure")
        };
        assert!(error.to_string().contains("gl"), "{error}");
    }

    #[test]
    fn opengl_driver_defaults() {
        assert_eq!(default_opengl_driver("linux").unwrap(), "libGL.so.1");
        assert_eq!(default_opengl_driver("win32").unwrap(), "opengl32.dll");
        assert!(default_opengl_driver("freebsd").is_err());
    }
}
