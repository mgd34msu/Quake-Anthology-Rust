//! Reference captures (donor `tools/reference/`).
//!
//! `platform` and `architecture` values use Node.js vocabulary
//! (`process.platform`/`process.arch`) so manifests stay comparable.

pub mod environment;
pub mod q1;
pub mod q1_retail;
pub mod q2;
pub mod q2_native;
pub mod q3;
pub mod q3_retail;
pub mod schema;
pub mod steam;

/// Operating system in `process.platform` vocabulary.
#[must_use]
pub fn node_platform() -> &'static str {
    match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    }
}

/// CPU architecture in `process.arch` vocabulary.
#[must_use]
pub fn node_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "x86" => "ia32",
        "aarch64" => "arm64",
        "powerpc64" => "ppc64",
        "powerpc" => "ppc",
        "mips64" => "mips64",
        "mips" => "mips",
        "s390x" => "s390x",
        "arm" => "arm",
        other => other,
    }
}
