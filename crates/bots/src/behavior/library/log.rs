//! Bot log from `src/bots/behavior/library/log.ts` (`be_interface.c`
//! `BotOpenLog`/`BotCloseLog`, `l_log.c` `Log_Write`/`Log_WriteTimeStamped`).
//!
//! One log file is open at a time; writes before open go to the print
//! sink. Timestamps render `MM:SS` from the donor's frame clock.

/// Log print severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BotLogSeverity {
    /// Informational message.
    Message,
    /// Warning.
    Warning,
    /// Error.
    Error,
}

/// Bot log sink.
pub trait BotLogSink {
    /// Print a message.
    fn print(&mut self, severity: BotLogSeverity, text: &str);
    /// Append bytes to the open log file.
    fn write(&mut self, bytes: &[u8]);
    /// Flush the open log file.
    fn flush(&mut self);
}

/// In-memory log sink for tests and headless hosts.
#[derive(Debug, Default)]
pub struct MemoryLogSink {
    /// Printed messages.
    pub messages: Vec<(BotLogSeverity, String)>,
    /// Written file bytes.
    pub file: Vec<u8>,
    /// Flush count.
    pub flushes: u32,
}

impl BotLogSink for MemoryLogSink {
    fn print(&mut self, severity: BotLogSeverity, text: &str) {
        self.messages.push((severity, text.to_owned()));
    }

    fn write(&mut self, bytes: &[u8]) {
        self.file.extend_from_slice(bytes);
    }

    fn flush(&mut self) {
        self.flushes += 1;
    }
}

/// Open bot log (`BotLog`).
#[derive(Debug)]
pub struct BotLog {
    filename: String,
    opened_filename: String,
    writes: u32,
    open: bool,
}

impl BotLog {
    /// Closed log.
    #[must_use]
    pub fn new() -> Self {
        Self {
            filename: String::new(),
            opened_filename: String::new(),
            writes: 0,
            open: false,
        }
    }

    /// Open (or reopen) the log file.
    pub fn open(&mut self, filename: Option<&str>, sink: &mut dyn BotLogSink) {
        if let Some(name) = filename {
            self.filename = name.to_owned();
        }
        self.opened_filename = self.filename.clone();
        self.writes = 0;
        self.open = true;
        sink.print(
            BotLogSeverity::Message,
            &format!("log opened: {}", self.opened_filename),
        );
    }

    /// Close the log file.
    pub fn close(&mut self, sink: &mut dyn BotLogSink) {
        if self.open {
            sink.flush();
            sink.print(
                BotLogSeverity::Message,
                &format!("log closed: {}", self.opened_filename),
            );
        }
        self.open = false;
    }

    /// Shut down the log (closes the file).
    pub fn shutdown(&mut self, sink: &mut dyn BotLogSink) {
        self.close(sink);
    }

    /// Whether a log file is open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Write a line.
    pub fn write(&mut self, formatted: &str, sink: &mut dyn BotLogSink) {
        if self.open {
            sink.write(formatted.as_bytes());
            sink.write(b"\n");
            self.writes += 1;
        } else {
            sink.print(BotLogSeverity::Message, formatted);
        }
    }

    /// Write a `MM:SS`-stamped line from the frame clock.
    pub fn write_time_stamped(&mut self, formatted: &str, time_seconds: f32, sink: &mut dyn BotLogSink) {
        let total = time_seconds.max(0.0) as u32;
        let line = format!("{:02}:{:02} {formatted}", total / 60, total % 60);
        self.write(&line, sink);
    }

    /// Flush the open file.
    pub fn flush(&self, sink: &mut dyn BotLogSink) {
        if self.open {
            sink.flush();
        }
    }

    /// Write count since open.
    #[must_use]
    pub fn write_count(&self) -> u32 {
        self.writes
    }

    /// Checkpoint the log position.
    #[must_use]
    pub fn checkpoint(&self) -> BotLogCheckpoint {
        BotLogCheckpoint {
            filename: self.filename.clone(),
            opened_filename: self.opened_filename.clone(),
            writes: self.writes,
            open: self.open,
        }
    }

    /// Restore a checkpoint.
    pub fn restore(&mut self, checkpoint: &BotLogCheckpoint) {
        self.filename = checkpoint.filename.clone();
        self.opened_filename = checkpoint.opened_filename.clone();
        self.writes = checkpoint.writes;
        self.open = checkpoint.open;
    }
}

impl Default for BotLog {
    fn default() -> Self {
        Self::new()
    }
}

/// Bot log checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotLogCheckpoint {
    /// Requested filename.
    pub filename: String,
    /// Opened filename.
    pub opened_filename: String,
    /// Writes since open.
    pub writes: u32,
    /// Whether a file is open.
    pub open: bool,
}
