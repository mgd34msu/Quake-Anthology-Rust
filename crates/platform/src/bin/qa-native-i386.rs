//! Companion process for admitted i386 native images; no engine startup.
fn main() {
    if cfg!(all(target_os = "linux", target_arch = "x86"))
        && let Some(status) = qa_platform::native::native_child_bootstrap()
    {
        std::process::exit(status);
    }
    std::process::exit(125);
}
