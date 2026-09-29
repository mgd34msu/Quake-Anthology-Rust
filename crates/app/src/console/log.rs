//! Append-only console log file.
//!
//! Donor provenance: `src/console/log.ts` (`ConsoleLog`). Same open modes
//! (append by default, truncate on request), same closed-write error, same
//! no-partial-write loop.

use std::fs::{File, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use super::ConsoleError;

/// Append-only UTF-8 console log.
#[derive(Debug)]
pub struct ConsoleLog {
    file: Option<File>,
    /// Log path.
    pub path: PathBuf,
}

impl ConsoleLog {
    /// Open (creating parents) `path` for append, or truncate when asked.
    pub fn open(path: &Path, append: bool) -> Result<Self, ConsoleError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|error| ConsoleError::BadLog(error.to_string()))?;
            }
        }
        let mut options = OpenOptions::new();
        options.create(true).write(true);
        if append {
            options.append(true);
        } else {
            options.truncate(true);
        }
        #[cfg(unix)]
        options.mode(0o600);
        let file = options
            .open(path)
            .map_err(|error| ConsoleError::BadLog(error.to_string()))?;
        Ok(Self {
            file: Some(file),
            path: path.to_path_buf(),
        })
    }

    /// Whether the log is still open.
    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.file.is_some()
    }

    /// Append text, writing until every byte lands.
    pub fn write(&mut self, text: &str) -> Result<(), ConsoleError> {
        let Some(file) = self.file.as_mut() else {
            return Err(ConsoleError::BadLog("Console log is closed".to_string()));
        };
        let bytes = text.as_bytes();
        let mut offset = 0;
        while offset < bytes.len() {
            match file.write(&bytes[offset..]) {
                Ok(0) => {
                    return Err(ConsoleError::BadLog("Console log write made no progress".to_string()));
                }
                Ok(written) => offset += written,
                Err(error) => return Err(ConsoleError::BadLog(error.to_string())),
            }
        }
        Ok(())
    }

    /// Close the log (idempotent).
    pub fn close(&mut self) {
        if let Some(mut file) = self.file.take() {
            let _ = file.flush();
        }
    }
}

impl Drop for ConsoleLog {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("qa-console-log-{}-{name}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn appends_truncates_and_rejects_closed_writes() {
        let path = scratch("append");
        let mut log = ConsoleLog::open(&path, true).unwrap();
        log.write("one\n").unwrap();
        log.close();
        let mut reopened = ConsoleLog::open(&path, true).unwrap();
        reopened.write("two\n").unwrap();
        reopened.close();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\n");
        let mut truncated = ConsoleLog::open(&path, false).unwrap();
        truncated.write("fresh\n").unwrap();
        truncated.close();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "fresh\n");
        assert!(truncated.write("late\n").is_err());
        let _ = std::fs::remove_file(&path);
    }
}
