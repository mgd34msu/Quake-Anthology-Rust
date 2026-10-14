//! Recorded system events use the formats codec and platform's owned files.
use qa_core::sys_events::{SysEvent, SysEventQueue};
use qa_formats::{FormatError, journal};
use qa_platform::files::{ReadFile, WriteFile};
use std::path::Path;

fn io(error: std::io::Error) -> FormatError {
    FormatError::Io(error.kind())
}

pub enum Mode<R = Recorder, P = Replay> {
    Recording(R),
    Replay(P),
}

pub struct Recorder {
    file: WriteFile,
    events: u64,
}
impl Recorder {
    /// Preserve existing recordings. Replacing one requires an explicit user
    /// file operation, rather than opening it as a side effect of startup.
    pub fn create(path: &Path, buffer_bytes: usize) -> Result<Self, FormatError> {
        let mut file = WriteFile::create_new(path, buffer_bytes).map_err(io)?;
        file.write_all(&journal::FILE_HEADER).map_err(io)?;
        Ok(Self { file, events: 0 })
    }
    pub fn append(&mut self, event: SysEvent<'_>) -> Result<(), FormatError> {
        self.file
            .write_all(&journal::encode_header(event)?)
            .map_err(io)?;
        self.file.write_all(event.kind.payload()).map_err(io)?;
        self.events = self.events.saturating_add(1);
        Ok(())
    }
    pub fn flush(&mut self) -> Result<(), FormatError> {
        self.file.flush().map_err(io)
    }
    pub fn events(&self) -> u64 {
        self.events
    }
}

pub struct Replay {
    file: ReadFile,
    end: u64,
    offset: u64,
    header: [u8; journal::RECORD_BYTES],
    payload: Box<[u8]>,
    pending: bool,
    events: u64,
}
impl Replay {
    pub fn open(path: &Path, payload_bytes: usize) -> Result<Self, FormatError> {
        let file = ReadFile::open(path).map_err(io)?;
        let mut header = [0; journal::FILE_HEADER.len()];
        file.read_exact_at(0, &mut header).map_err(io)?;
        journal::check_file_header(&header)?;
        let end = file.length().map_err(io)?;
        let mut payload = Vec::new();
        payload
            .try_reserve_exact(payload_bytes)
            .map_err(|_| FormatError::InvalidRange)?;
        payload.resize(payload_bytes, 0);
        Ok(Self {
            file,
            end,
            offset: journal::FILE_HEADER.len() as u64,
            header: [0; journal::RECORD_BYTES],
            payload: payload.into_boxed_slice(),
            pending: false,
            events: 0,
        })
    }
    /// A pending record retains its borrowed payload across queue backpressure.
    /// Nothing advances until the caller acknowledges successful admission.
    pub fn peek(&mut self) -> Result<Option<SysEvent<'_>>, FormatError> {
        if !self.pending {
            if self.offset == self.end {
                return Ok(None);
            }
            let payload_at = self
                .offset
                .checked_add(journal::RECORD_BYTES as u64)
                .ok_or(FormatError::InvalidRange)?;
            if payload_at > self.end {
                return Err(FormatError::Truncated);
            }
            self.file
                .read_exact_at(self.offset, &mut self.header)
                .map_err(io)?;
            let length = journal::payload_length(&self.header);
            if length > self.payload.len() {
                return Err(FormatError::InvalidRange);
            }
            let end = payload_at
                .checked_add(length as u64)
                .ok_or(FormatError::InvalidRange)?;
            if end > self.end {
                return Err(FormatError::Truncated);
            }
            self.file
                .read_exact_at(payload_at, &mut self.payload[..length])
                .map_err(io)?;
            self.pending = true;
        }
        journal::decode(
            &self.header,
            &self.payload[..journal::payload_length(&self.header)],
        )
        .map(Some)
    }
    pub fn advance(&mut self) {
        if self.pending {
            self.offset +=
                journal::RECORD_BYTES as u64 + journal::payload_length(&self.header) as u64;
            self.pending = false;
            self.events = self.events.saturating_add(1);
        }
    }
    pub fn events(&self) -> u64 {
        self.events
    }
    /// Called at an existing host intake. The queue keeps its reserved TIME
    /// slot, and the next intake resumes from an unadmitted record.
    pub fn enqueue_drain(&mut self, queue: &mut SysEventQueue) -> Result<bool, FormatError> {
        let first = self.events;
        loop {
            let Some(event) = self.peek()? else {
                return if self.events == first {
                    Ok(false)
                } else {
                    Err(FormatError::Truncated)
                };
            };
            let end_drain = matches!(event.kind, qa_core::sys_events::EventKind::Time);
            if queue.push(event).is_err() {
                return Ok(true);
            }
            self.advance();
            if end_drain {
                return Ok(true);
            }
        }
    }
}
