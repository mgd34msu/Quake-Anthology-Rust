/// Current free physical memory, matching the TypeScript host's freemem query.
/// Unknown OS support is distinct from a successful zero-memory reading.
pub fn available_memory() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        use std::ffi::{c_int, c_long};
        unsafe extern "C" {
            fn sysconf(name: c_int) -> c_long;
        }
        // Linux libc's _SC_PAGESIZE and _SC_AVPHYS_PAGES. Neither call takes
        // pointers or lends memory; -1 reports an unavailable system limit.
        // SAFETY: Both selectors are valid scalar sysconf arguments.
        let (page_bytes, pages) = unsafe { (sysconf(30), sysconf(86)) };
        u64::try_from(page_bytes)
            .ok()?
            .checked_mul(u64::try_from(pages).ok()?)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}
