#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "native/session.rs"]
mod session;

fn main() {
    if let Some(status) = qa_platform::native::native_child_bootstrap() {
        std::process::exit(status);
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    session::run();
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    println!("native child session checks unavailable on this host");
}
