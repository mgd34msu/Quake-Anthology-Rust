//! Execution isolation (donor `tools/verify/isolation.ts`).
//!
//! Loopback port leases, pinned input staging, and private X displays.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::error::ToolsError;
use crate::fsutil::copy_exclusive;
use crate::json::escape_string;
use crate::sys::{detach, kill_process_group, owner_uid, SIGKILL};
use crate::verify::hash::hash_file;
use crate::verify::snapshot::resolve;

/// A leased set of loopback ports with on-disk locks.
pub struct PortLease {
    /// Leased ports.
    pub ports: Vec<u16>,
    listeners: Vec<TcpListener>,
    locks: Vec<PathBuf>,
}

impl PortLease {
    /// Release the sockets so the observed process can bind the ports.
    pub fn release_sockets(&mut self) {
        self.listeners.clear();
    }

    /// Release sockets and remove lock files.
    pub fn dispose(mut self) {
        self.listeners.clear();
        for lock in self.locks.drain(..) {
            let _ = fs::remove_file(lock);
        }
    }
}

impl Drop for PortLease {
    fn drop(&mut self) {
        for lock in self.locks.drain(..) {
            let _ = fs::remove_file(lock);
        }
    }
}

/// Lease `count` loopback ports owned by `owner`.
pub fn lease_ports(count: usize, owner: &str) -> Result<PortLease, ToolsError> {
    let root = std::env::temp_dir().join(format!("quake-verify-ports-{}", owner_uid()));
    if !root.exists() {
        fs::create_dir_all(&root).map_err(|error| ToolsError::io(format!("creating {}", root.display()), error))?;
        set_permissions(&root, 0o700)?;
    }
    let mut listeners = Vec::new();
    let mut ports = Vec::new();
    let mut locks = Vec::new();
    let mut lease = PortLease {
        ports: Vec::new(),
        listeners: Vec::new(),
        locks: Vec::new(),
    };
    let result = (|| -> Result<(), ToolsError> {
        let mut collisions = 0;
        while ports.len() < count {
            let listener =
                TcpListener::bind("127.0.0.1:0").map_err(|error| ToolsError::io("leasing a loopback port", error))?;
            let port = listener
                .local_addr()
                .map_err(|error| ToolsError::io("leasing a loopback port", error))?
                .port();
            let path = root.join(format!("{port}.json"));
            let mut owner_text = String::new();
            escape_string(owner, &mut owner_text);
            let lock_text = format!(
                "{{\"owner\":{owner_text},\"pid\":{},\"port\":{port}}}",
                std::process::id()
            );
            match exclusive_text(&path, &lock_text, 0o600) {
                Ok(()) => {}
                Err(error) => {
                    drop(listener);
                    if is_exists(&error) && collisions < 128 {
                        collisions += 1;
                        continue;
                    }
                    return Err(error);
                }
            }
            locks.push(path);
            ports.push(port);
            listeners.push(listener);
        }
        Ok(())
    })();
    if let Err(error) = result {
        for lock in locks.drain(..) {
            let _ = fs::remove_file(lock);
        }
        return Err(error);
    }
    lease.ports = ports;
    lease.listeners = listeners;
    lease.locks = locks;
    Ok(lease)
}

fn is_exists(error: &ToolsError) -> bool {
    matches!(error, ToolsError::Io { source, .. } if source.kind() == std::io::ErrorKind::AlreadyExists)
}

fn exclusive_text(path: &Path, text: &str, mode: u32) -> Result<(), ToolsError> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode);
    }
    let _ = mode;
    let mut file = options
        .open(path)
        .map_err(|error| ToolsError::io(format!("locking {}", path.display()), error))?;
    file.write_all(text.as_bytes())
        .map_err(|error| ToolsError::io(format!("locking {}", path.display()), error))?;
    Ok(())
}

#[cfg(unix)]
fn set_permissions(path: &Path, mode: u32) -> Result<(), ToolsError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|error| ToolsError::io(format!("setting mode on {}", path.display()), error))
}

#[cfg(not(unix))]
fn set_permissions(_path: &Path, _mode: u32) -> Result<(), ToolsError> {
    Ok(())
}

/// Copy `source` to `destination` exclusively, verify its hash, and lock its mode.
///
/// Refuses symlinked sources and rehashes after the copy.
pub fn copy_pinned_file(source: &Path, destination: &Path, sha256: &str, executable: bool) -> Result<(), ToolsError> {
    let canonical =
        fs::canonicalize(source).map_err(|error| ToolsError::io(format!("resolving {}", source.display()), error))?;
    if canonical != resolve(source) {
        return Err(ToolsError::invalid(format!(
            "Input symlink is not an owned immutable file: {}",
            source.display()
        )));
    }
    copy_exclusive(source, destination)?;
    set_permissions(destination, if executable { 0o500 } else { 0o400 })?;
    if hash_file(destination)? != sha256 {
        return Err(ToolsError::invalid(format!(
            "Input changed while being copied: {}",
            source.display()
        )));
    }
    Ok(())
}

/// A private X display with its provider process.
pub struct PrivateDisplay {
    /// Display name (`:N`).
    pub display: String,
    child: Option<Child>,
}

impl PrivateDisplay {
    /// Kill the provider group and reap it.
    pub fn dispose(mut self) {
        if let Some(mut child) = self.child.take() {
            if !kill_process_group(child.id(), SIGKILL) {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

impl Drop for PrivateDisplay {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            if !kill_process_group(child.id(), SIGKILL) {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

/// Start a private display provider (`Xvfb`-shaped) writing its number to `display.txt`.
pub fn start_private_display(
    executable: &Path,
    output_root: &Path,
    environment: &HashMap<String, String>,
) -> Result<PrivateDisplay, ToolsError> {
    let display_path = output_root.join("display.txt");
    let stdout = File::create(&display_path)
        .map_err(|error| ToolsError::io(format!("creating {}", display_path.display()), error))?;
    let stderr_path = output_root.join("display-stderr.txt");
    let stderr = File::create(&stderr_path)
        .map_err(|error| ToolsError::io(format!("creating {}", stderr_path.display()), error))?;
    let mut command = Command::new(executable);
    command
        .args(["-displayfd", "1", "-screen", "0", "1280x720x24", "-nolisten", "tcp"])
        .env_clear()
        .envs(environment)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    detach(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| ToolsError::io(format!("starting {}", executable.display()), error))?;
    let deadline = Instant::now() + Duration::from_millis(5000);
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| ToolsError::io("polling display provider", error))?
        {
            let code = status.code().map_or(-1, |code| code as i64);
            kill_process_group(child.id(), SIGKILL);
            let _ = child.wait();
            return Err(ToolsError::invalid(format!("Private display provider exited {code}")));
        }
        let text = fs::read_to_string(&display_path).unwrap_or_default();
        if is_display_number(&text) {
            return Ok(PrivateDisplay {
                display: format!(":{}", text.trim()),
                child: Some(child),
            });
        }
        if Instant::now() >= deadline {
            kill_process_group(child.id(), SIGKILL);
            let _ = child.wait();
            return Err(ToolsError::invalid(
                "Private display provider did not allocate a display within 5000ms",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn is_display_number(text: &str) -> bool {
    text.ends_with('\n')
        && !text.is_empty()
        && text[..text.len() - 1].bytes().all(|byte| byte.is_ascii_digit())
        && !text[..text.len() - 1].is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_number_shape() {
        assert!(is_display_number("3\n"));
        assert!(!is_display_number("3"));
        assert!(!is_display_number("\n"));
        assert!(!is_display_number("3\n4\n"));
        assert!(!is_display_number("x\n"));
    }

    #[test]
    fn port_lease_round_trip() {
        let mut lease = lease_ports(2, "test-owner").unwrap();
        assert_eq!(lease.ports.len(), 2);
        assert_ne!(lease.ports[0], lease.ports[1]);
        lease.release_sockets();
        lease.dispose();
    }
}
