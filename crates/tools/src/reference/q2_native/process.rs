//! Observed child processes (donor `tools/reference/q2-native/process.ts`).
//!
//! Spawns a child with a replaced environment and piped stdio, streams both
//! outputs on reader threads, and supports stdin commands plus checkpoint
//! waits. A watchdog kills runs past their timeout; `finish` reaps with
//! SIGTERM, escalates to SIGKILL after two seconds, and writes the captured
//! streams beside the observation.

use std::io::Write;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::error::ToolsError;
use crate::fsutil;
use crate::json::Json;
use crate::process::{snapshot, spawn_reader, SharedBytes};
use crate::sys::{kill_process, SIGKILL, SIGTERM};
use crate::time::now_iso;

/// Kind of an observed process event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// Text written to stdin.
    Stdin,
    /// Checkpoint text observed in output.
    Checkpoint,
}

impl EventKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Stdin => "stdin",
            Self::Checkpoint => "checkpoint",
        }
    }
}

/// An observed stdin write or checkpoint.
#[derive(Debug, Clone)]
pub struct ProcessEvent {
    /// Milliseconds from spawn to the event.
    pub elapsed_ms: f64,
    /// Event kind.
    pub kind: EventKind,
    /// Written or awaited text.
    pub text: String,
}

impl ProcessEvent {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("elapsedMs".to_owned(), Json::float(self.elapsed_ms)),
            ("kind".to_owned(), Json::string(self.kind.as_str())),
            ("text".to_owned(), Json::string(&self.text)),
        ])
    }
}

/// A finished observation: identity, timing, streams, and events.
#[derive(Debug, Clone)]
pub struct ProcessObservation {
    /// Argument vector.
    pub command: Vec<String>,
    /// Working directory.
    pub cwd: String,
    /// Replaced environment in caller order.
    pub environment: Vec<(String, String)>,
    /// Child pid.
    pub pid: u32,
    /// ISO-8601 start time.
    pub started_at: String,
    /// Elapsed milliseconds.
    pub duration_ms: f64,
    /// Exit code (`None` when killed by a signal).
    pub exit_code: Option<i32>,
    /// Whether the watchdog fired.
    pub timeout: bool,
    /// Observed events.
    pub events: Vec<ProcessEvent>,
    /// Captured standard output.
    pub stdout: String,
    /// Captured standard error.
    pub stderr: String,
}

impl ProcessObservation {
    /// Render with donor field order.
    #[must_use]
    pub fn to_json(&self) -> Json {
        Json::object(vec![
            ("command".to_owned(), Json::array(self.command.iter().map(Json::string).collect())),
            ("cwd".to_owned(), Json::string(&self.cwd)),
            (
                "environment".to_owned(),
                Json::object(self.environment.iter().map(|(key, value)| (key.clone(), Json::string(value))).collect()),
            ),
            ("pid".to_owned(), Json::uint(u64::from(self.pid))),
            ("startedAt".to_owned(), Json::string(&self.started_at)),
            ("durationMs".to_owned(), Json::float(self.duration_ms)),
            ("exitCode".to_owned(), self.exit_code.map_or(Json::Null, |code| Json::int(i64::from(code)))),
            ("timeout".to_owned(), Json::boolean(self.timeout)),
            ("events".to_owned(), Json::array(self.events.iter().map(ProcessEvent::to_json).collect())),
            ("stdout".to_owned(), Json::string(&self.stdout)),
            ("stderr".to_owned(), Json::string(&self.stderr)),
            ("cleanup".to_owned(), Json::string("reaped")),
        ])
    }
}

/// A spawned, observed child process.
pub struct ObservedProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: SharedBytes,
    stderr: SharedBytes,
    readers: Vec<JoinHandle<()>>,
    command: Vec<String>,
    cwd: String,
    environment: Vec<(String, String)>,
    started_at: String,
    start: Instant,
    events: Vec<ProcessEvent>,
    stopped: Arc<AtomicBool>,
    timed_out: Arc<AtomicBool>,
    watchdog: Option<JoinHandle<()>>,
}

impl ObservedProcess {
    /// Child pid.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    fn elapsed_ms(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }

    /// Standard output plus standard error observed so far.
    #[must_use]
    pub fn output(&self) -> String {
        snapshot(&self.stdout) + &snapshot(&self.stderr)
    }

    fn exited(&mut self) -> Result<bool, ToolsError> {
        self.child.try_wait().map(|status| status.is_some()).map_err(|error| ToolsError::io("polling a child process", error))
    }

    /// Whether the child is still running.
    pub fn alive(&mut self) -> Result<bool, ToolsError> {
        self.exited().map(|exited| !exited)
    }

    /// Wait for natural exit and return the exit code (`None` on signals).
    pub fn wait_for_exit(&mut self) -> Result<Option<i32>, ToolsError> {
        let status = self.child.wait().map_err(|error| ToolsError::io("waiting for a child process", error))?;
        self.stopped.store(true, Ordering::SeqCst);
        Ok(status.code())
    }

    /// Send one line to stdin; fails when the process already exited.
    pub fn send(&mut self, text: &str) -> Result<(), ToolsError> {
        if self.exited()? {
            return Err(ToolsError::invalid(format!("Process exited before command: {text}")));
        }
        self.events.push(ProcessEvent { elapsed_ms: self.elapsed_ms(), kind: EventKind::Stdin, text: text.to_owned() });
        let stdin = self.stdin.as_mut().ok_or_else(|| ToolsError::invalid(format!("Process exited before command: {text}")))?;
        stdin
            .write_all(format!("{text}\n").as_bytes())
            .and_then(|()| stdin.flush())
            .map_err(|error| ToolsError::io("writing to child stdin", error))
    }

    /// Wait until either stream contains `text`; `timeout_ms` defaults to 15 seconds.
    pub fn wait_for(&mut self, text: &str, timeout_ms: Option<u64>) -> Result<(), ToolsError> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms.unwrap_or(15_000));
        loop {
            let stdout = snapshot(&self.stdout);
            let stderr = snapshot(&self.stderr);
            if stdout.contains(text) || stderr.contains(text) {
                self.events.push(ProcessEvent { elapsed_ms: self.elapsed_ms(), kind: EventKind::Checkpoint, text: text.to_owned() });
                return Ok(());
            }
            if self.exited()? {
                return Err(ToolsError::invalid(format!("Process exited before {text}: {stdout}{stderr}")));
            }
            if Instant::now() > deadline {
                return Err(ToolsError::invalid(format!(
                    "Missing checkpoint {text}: {}{}",
                    tail_chars(&stdout, 2500),
                    tail_chars(&stderr, 1000)
                )));
            }
            thread::sleep(Duration::from_millis(25));
        }
    }

    /// Reap the process, write its streams under `directory`, and observe it.
    pub fn finish(&mut self, directory: &str) -> Result<ProcessObservation, ToolsError> {
        if !self.stopped.swap(true, Ordering::SeqCst) && !self.exited()? {
            kill_process(self.child.id(), SIGTERM);
        }
        let force_at = Instant::now() + Duration::from_millis(2000);
        let status = loop {
            match self.child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if Instant::now() >= force_at {
                        kill_process(self.child.id(), SIGKILL);
                    }
                    thread::sleep(Duration::from_millis(25));
                }
                Err(error) => return Err(ToolsError::io("reaping a child process", error)),
            }
        };
        if let Some(watchdog) = self.watchdog.take() {
            let _ = watchdog.join();
        }
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
        let stdout = snapshot(&self.stdout);
        let stderr = snapshot(&self.stderr);
        std::fs::create_dir_all(directory)
            .map_err(|error| ToolsError::io(format!("creating {directory}"), error))?;
        fsutil::write_text(std::path::Path::new(directory).join("stdout.txt").as_path(), &stdout)?;
        fsutil::write_text(std::path::Path::new(directory).join("stderr.txt").as_path(), &stderr)?;
        Ok(ProcessObservation {
            command: self.command.clone(),
            cwd: self.cwd.clone(),
            environment: self.environment.clone(),
            pid: self.child.id(),
            started_at: self.started_at.clone(),
            duration_ms: self.elapsed_ms(),
            exit_code: status.code(),
            timeout: self.timed_out.load(Ordering::SeqCst),
            events: std::mem::take(&mut self.events),
            stdout,
            stderr,
        })
    }
}

/// Last `count` characters of `text`.
fn tail_chars(text: &str, count: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    chars[chars.len().saturating_sub(count)..].iter().collect()
}

/// Spawn an observed child; the watchdog SIGKILLs past `timeout_ms` (default 60 seconds).
pub fn start_observed(
    command: &[String],
    cwd: &str,
    environment: &[(String, String)],
    timeout_ms: Option<u64>,
) -> Result<ObservedProcess, ToolsError> {
    let (program, args) = command.split_first().ok_or_else(|| ToolsError::invalid("Cannot observe an empty command"))?;
    let mut child_command = Command::new(program);
    child_command.args(args).current_dir(cwd).env_clear().envs(environment.iter().cloned());
    child_command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = child_command
        .spawn()
        .map_err(|error| ToolsError::io(format!("spawning {}", command.join(" ")), error))?;
    let stdin = child.stdin.take();
    let (stdout, out_handle) = spawn_reader(child.stdout.take().expect("piped stdout"));
    let (stderr, err_handle) = spawn_reader(child.stderr.take().expect("piped stderr"));
    let stopped = Arc::new(AtomicBool::new(false));
    let timed_out = Arc::new(AtomicBool::new(false));
    let pid = child.id();
    let timeout = timeout_ms.unwrap_or(60_000);
    let watchdog_stopped = Arc::clone(&stopped);
    let watchdog_timed_out = Arc::clone(&timed_out);
    let watchdog = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_millis(timeout);
        while Instant::now() < deadline {
            if watchdog_stopped.load(Ordering::SeqCst) {
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        if watchdog_stopped.load(Ordering::SeqCst) {
            return;
        }
        watchdog_timed_out.store(true, Ordering::SeqCst);
        kill_process(pid, SIGKILL);
    });
    Ok(ObservedProcess {
        child,
        stdin,
        stdout,
        stderr,
        readers: vec![out_handle, err_handle],
        command: command.to_vec(),
        cwd: cwd.to_owned(),
        environment: environment.to_vec(),
        started_at: now_iso(),
        start: Instant::now(),
        events: Vec::new(),
        stopped,
        timed_out,
        watchdog: Some(watchdog),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment() -> Vec<(String, String)> {
        vec![("PATH".to_owned(), "/usr/bin:/bin".to_owned()), ("LC_ALL".to_owned(), "C".to_owned())]
    }

    fn temp_dir(prefix: &str) -> String {
        fsutil::make_temp_dir(&std::env::temp_dir(), prefix).expect("temp dir").to_string_lossy().into_owned()
    }

    #[test]
    fn observes_checkpoints_and_reaps() {
        let directory = temp_dir("quake-observed-");
        let mut child = start_observed(
            &[String::from("sh"), String::from("-c"), String::from("echo READY; exec sleep 30")],
            &directory,
            &environment(),
            None,
        )
        .expect("spawn");
        child.wait_for("READY", None).expect("checkpoint");
        assert!(child.output().contains("READY"));
        let observation = child.finish(&format!("{directory}/logs")).expect("finish");
        assert_eq!(observation.command.len(), 3);
        assert_eq!(observation.cwd, directory);
        assert!(observation.exit_code.is_none(), "signaled: {:?}", observation.exit_code);
        assert!(!observation.timeout);
        assert_eq!(observation.events.len(), 1);
        assert_eq!(observation.stdout, "READY\n");
        let rendered = observation.to_json();
        assert_eq!(rendered.get("cleanup").and_then(Json::as_str), Some("reaped"));
        fsutil::remove_forced(std::path::Path::new(&directory));
    }

    #[test]
    fn sends_stdin_and_waits_for_exit() {
        let directory = temp_dir("quake-observed-stdin-");
        let mut child = start_observed(
            &[String::from("sh"), String::from("-c"), String::from("read line; echo got:$line")],
            &directory,
            &environment(),
            None,
        )
        .expect("spawn");
        child.send("hello").expect("send");
        assert_eq!(child.wait_for_exit().expect("exit"), Some(0));
        let observation = child.finish(&format!("{directory}/logs")).expect("finish");
        assert_eq!(observation.stdout, "got:hello\n");
        assert!(observation.events.iter().any(|event| event.kind == EventKind::Stdin && event.text == "hello"));
        fsutil::remove_forced(std::path::Path::new(&directory));
    }

    #[test]
    fn reports_missing_checkpoints_and_exited_sends() {
        let directory = temp_dir("quake-observed-missing-");
        let mut child = start_observed(
            &[String::from("sh"), String::from("-c"), String::from("echo READY; exec sleep 30")],
            &directory,
            &environment(),
            None,
        )
        .expect("spawn");
        let error = child.wait_for("NEVER", Some(100)).expect_err("missing checkpoint");
        assert!(error.to_string().starts_with("Missing checkpoint NEVER:"), "{error}");
        let _ = child.finish(&format!("{directory}/logs")).expect("finish");
        let error = child.send("late").expect_err("send after reap");
        assert!(error.to_string().starts_with("Process exited before command:"), "{error}");
        fsutil::remove_forced(std::path::Path::new(&directory));
    }

    #[test]
    fn watchdog_kills_overtime_processes() {
        let directory = temp_dir("quake-observed-watchdog-");
        let mut child = start_observed(
            &[String::from("sh"), String::from("-c"), String::from("exec sleep 30")],
            &directory,
            &environment(),
            Some(200),
        )
        .expect("spawn");
        thread::sleep(Duration::from_millis(400));
        let observation = child.finish(&format!("{directory}/logs")).expect("finish");
        assert!(observation.timeout);
        assert!(observation.exit_code.is_none());
        fsutil::remove_forced(std::path::Path::new(&directory));
    }
}
