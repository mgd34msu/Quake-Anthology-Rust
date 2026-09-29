//! Subprocess helpers: PATH lookup, streaming capture, and timeouts.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::error::ToolsError;
use crate::sys::{kill_process_group, SIGKILL};

/// Find `name` on `PATH` (donor `Bun.which`).
#[must_use]
pub fn which(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        let path = PathBuf::from(name);
        return is_executable(&path).then_some(path);
    }
    let paths = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&paths) {
        let candidate = directory.join(name);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file() && path.metadata().is_ok_and(|meta| meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// Bytes collected by a reader thread.
pub type SharedBytes = Arc<Mutex<Vec<u8>>>;

/// Drain `stream` to EOF on a background thread, collecting bytes.
pub fn spawn_reader<R: Read + Send + 'static>(stream: R) -> (SharedBytes, JoinHandle<()>) {
    let shared: SharedBytes = Arc::new(Mutex::new(Vec::new()));
    let target = Arc::clone(&shared);
    let handle = thread::spawn(move || {
        let mut stream = stream;
        let mut chunk = [0u8; 8192];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(count) => {
                    if let Ok(mut guard) = target.lock() {
                        guard.extend_from_slice(&chunk[..count]);
                    }
                }
                Err(_) => break,
            }
        }
    });
    (shared, handle)
}

/// Snapshot collected bytes as lossy text.
#[must_use]
pub fn snapshot(shared: &SharedBytes) -> String {
    shared
        .lock()
        .map(|guard| String::from_utf8_lossy(&guard).into_owned())
        .unwrap_or_default()
}

/// Wait for `child` up to `timeout_ms`, killing its group on expiry.
///
/// Returns `(timed_out, exit_code)`.
pub fn wait_timeout_group(child: &mut Child, timeout_ms: u64) -> Result<(bool, Option<i32>), ToolsError> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok((false, status.code())),
            Ok(None) => {}
            Err(error) => return Err(ToolsError::io("waiting for child process", error)),
        }
        if Instant::now() >= deadline {
            kill_process_group(child.id(), SIGKILL);
            let _ = child.kill();
            let _ = child.wait();
            let code = child.try_wait().unwrap_or(None).and_then(|status| status.code());
            return Ok((true, code));
        }
        thread::sleep(Duration::from_millis(5));
    }
}

/// How a spawned command receives its environment.
pub enum EnvSpec {
    /// Inherit the whole environment.
    Inherit,
    /// Inherit plus overrides.
    InheritWith(HashMap<String, String>),
    /// Exactly these variables.
    Replace(HashMap<String, String>),
}

/// A finished capture: streams, exit code, and timeout flag.
pub struct Completed {
    /// Captured standard output.
    pub stdout: String,
    /// Captured standard error.
    pub stderr: String,
    /// Exit code (`None` when killed by a signal).
    pub exit_code: Option<i32>,
    /// Whether the timeout fired.
    pub timed_out: bool,
}

/// Spawn `argv` in `cwd`, capture both streams, and enforce `timeout_ms`.
///
/// Reader threads start before waiting so partial output survives timeouts.
pub fn run_capture(
    argv: &[String],
    cwd: &Path,
    env: &EnvSpec,
    timeout_ms: u64,
) -> Result<Completed, ToolsError> {
    let (program, args) = argv.split_first().ok_or_else(|| ToolsError::invalid("Cannot run an empty command"))?;
    let mut command = Command::new(program);
    command.args(args).current_dir(cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    match env {
        EnvSpec::Inherit => {}
        EnvSpec::InheritWith(extra) => {
            command.envs(extra);
        }
        EnvSpec::Replace(vars) => {
            command.env_clear().envs(vars);
        }
    }
    let mut child = command
        .spawn()
        .map_err(|error| ToolsError::io(format!("spawning {}", argv.join(" ")), error))?;
    let (out_bytes, out_handle) = spawn_reader(child.stdout.take().expect("piped stdout"));
    let (err_bytes, err_handle) = spawn_reader(child.stderr.take().expect("piped stderr"));
    let (timed_out, exit_code) = wait_timeout_group(&mut child, timeout_ms)?;
    let _ = out_handle.join();
    let _ = err_handle.join();
    Ok(Completed {
        stdout: snapshot(&out_bytes),
        stderr: snapshot(&err_bytes),
        exit_code,
        timed_out,
    })
}
