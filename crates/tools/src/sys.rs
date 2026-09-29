//! Small Unix process-control primitives (no extra crates).
//!
//! Signal delivery and session detachment go through direct `libc` symbols so
//! observed processes can be reaped as groups like the donor `detached` trees.

/// Terminate signal number.
pub const SIGTERM: i32 = 15;
/// Kill signal number.
pub const SIGKILL: i32 = 9;

#[cfg(unix)]
mod unix {
    use std::os::raw::c_int;

    #[link(name = "c")]
    extern "C" {
        fn kill(pid: c_int, signal: c_int) -> c_int;
        fn getuid() -> u32;
        fn setsid() -> c_int;
    }

    /// Send `signal` to a process group leader's whole group (`kill(-pid)`).
    pub fn kill_process_group(pid: u32, signal: i32) -> bool {
        if pid == 0 || pid > i32::MAX as u32 {
            return false;
        }
        // SAFETY: `kill` with a negative pid and a valid signal number only
        // signals an existing group; failures surface as a nonzero return.
        unsafe { kill(-(pid as c_int), signal as c_int) == 0 }
    }

    /// Current user id, or `None` when unavailable.
    pub fn current_uid() -> Option<u32> {
        // SAFETY: `getuid` takes no arguments and always succeeds.
        Some(unsafe { getuid() })
    }

    /// Detach a child into its own session (donor `detached: true`).
    pub fn detach(command: &mut std::process::Command) {
        use std::os::unix::process::CommandExt;
        // SAFETY: `setsid` is async-signal-safe; the closure runs after fork
        // before exec with no locks held by the child.
        unsafe {
            command.pre_exec(|| {
                setsid();
                Ok(())
            });
        }
    }
}

#[cfg(not(unix))]
mod unix {
    /// Non-Unix fallback: group signalling is unsupported.
    pub fn kill_process_group(_pid: u32, _signal: i32) -> bool {
        false
    }

    /// Non-Unix fallback: no user id is available.
    pub fn current_uid() -> Option<u32> {
        None
    }

    /// Non-Unix fallback: detachment is a no-op.
    pub fn detach(_command: &mut std::process::Command) {}
}

pub use unix::{current_uid, detach, kill_process_group};

/// Owner label for port leases (`process.getuid() ?? "user"` donor shape).
#[must_use]
pub fn owner_uid() -> String {
    current_uid().map_or_else(|| "user".to_owned(), |uid| uid.to_string())
}
