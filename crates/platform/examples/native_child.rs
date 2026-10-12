//! Native-process fixture bootstrap, before libtest can create worker threads.
fn main() {
    #[cfg(target_os = "linux")]
    if std::env::var_os("QA_NATIVE_TEST_SIGNAL").is_some() {
        unsafe extern "C" {
            fn signal(number: i32, handler: usize) -> usize;
        }
        extern "C" fn caught(_: i32) {}
        // SAFETY: this standalone, owned fixture has one thread. The handler
        // touches no memory and exists only to prove notification deferral.
        if unsafe { signal(10, caught as *const () as usize) } == usize::MAX {
            std::process::exit(125);
        }
    }
    if let Some(status) = qa_platform::native::native_child_bootstrap() {
        std::process::exit(status);
    }
}
