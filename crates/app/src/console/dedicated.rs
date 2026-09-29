//! Dedicated-server stdin console.
//!
//! Donor provenance: `src/console/dedicated.ts` (`DedicatedConsole`).
//! Dedicated stdin is independent of windowing input and retains partial
//! UTF-8 sequences and partial lines between polls. The async stream
//! becomes explicit [`DedicatedConsole::feed_bytes`]/[`DedicatedConsole::end`]
//! calls; [`DedicatedConsole::drain`] hands complete lines to the caller
//! instead of appending to a shared command buffer.

use std::collections::VecDeque;

use super::ConsoleError;

/// Line-buffered stdin reader with retained partial input.
#[derive(Debug, Default)]
pub struct DedicatedConsole {
    pending_bytes: Vec<u8>,
    pending: String,
    lines: VecDeque<String>,
    ended: bool,
    closed: bool,
    error: Option<String>,
}

impl DedicatedConsole {
    /// Open a fresh reader.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed raw stdin bytes, retaining incomplete trailing UTF-8.
    pub fn feed_bytes(&mut self, bytes: &[u8]) {
        self.pending_bytes.extend_from_slice(bytes);
        self.decode();
        self.split();
    }

    /// Feed already-decoded stdin text.
    pub fn feed_str(&mut self, text: &str) {
        self.pending.push_str(text);
        self.split();
    }

    /// Mark end-of-input, flushing the decoder and any partial line.
    pub fn end(&mut self) {
        let tail = String::from_utf8_lossy(&self.pending_bytes).into_owned();
        self.pending_bytes.clear();
        self.pending.push_str(&tail);
        self.split();
        if !self.pending.is_empty() {
            let line = std::mem::take(&mut self.pending);
            self.lines.push_back(line);
        }
        self.ended = true;
    }

    /// Record a stream error, rethrown by [`DedicatedConsole::drain`].
    pub fn fail(&mut self, error: String) {
        self.error = Some(error);
    }

    /// Whether input ended and every line drained.
    #[must_use]
    pub fn eof(&self) -> bool {
        self.ended && self.lines.is_empty()
    }

    /// Hand complete lines to `sink`, returning the line count.
    pub fn drain(&mut self, sink: &mut dyn FnMut(&str)) -> Result<usize, ConsoleError> {
        if self.closed {
            return Err(ConsoleError::BadDedicated("Dedicated console is closed".to_string()));
        }
        if let Some(error) = self.error.clone() {
            return Err(ConsoleError::BadDedicated(error));
        }
        let mut count = 0;
        while let Some(line) = self.lines.pop_front() {
            sink(&line);
            count += 1;
        }
        Ok(count)
    }

    /// Close the reader (idempotent).
    pub const fn close(&mut self) {
        self.closed = true;
    }

    fn decode(&mut self) {
        loop {
            if self.pending_bytes.is_empty() {
                return;
            }
            match std::str::from_utf8(&self.pending_bytes) {
                Ok(valid) => {
                    self.pending.push_str(valid);
                    self.pending_bytes.clear();
                    return;
                }
                Err(error) => {
                    let valid_up_to = error.valid_up_to();
                    let valid: Vec<u8> = self.pending_bytes.drain(..valid_up_to).collect();
                    self.pending.push_str(&String::from_utf8_lossy(&valid));
                    match error.error_len() {
                        Some(length) => {
                            self.pending.push('\u{FFFD}');
                            self.pending_bytes.drain(..length);
                        }
                        None => return,
                    }
                }
            }
        }
    }

    fn split(&mut self) {
        while let Some(index) = self.pending.find('\n') {
            let mut line: String = self.pending.drain(..=index).collect();
            line.pop();
            if let Some(stripped) = line.strip_suffix('\r') {
                line = stripped.to_string();
            }
            self.lines.push_back(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_partial_lines_and_utf8() {
        let mut console = DedicatedConsole::new();
        console.feed_bytes("say hel".as_bytes());
        console.feed_bytes(b"lo\nsay \xe4\xb8");
        let mut drained = Vec::new();
        assert_eq!(console.drain(&mut |line| drained.push(line.to_string())).unwrap(), 1);
        assert_eq!(drained, vec!["say hello".to_string()]);
        console.feed_bytes(b"\xad\r\npartial");
        let mut drained = Vec::new();
        assert_eq!(console.drain(&mut |line| drained.push(line.to_string())).unwrap(), 1);
        assert_eq!(drained, vec!["say \u{4e2d}".to_string()]);
        console.end();
        let mut drained = Vec::new();
        assert_eq!(console.drain(&mut |line| drained.push(line.to_string())).unwrap(), 1);
        assert_eq!(drained, vec!["partial".to_string()]);
        assert!(console.eof());
    }

    #[test]
    fn close_and_error_block_drain() {
        let mut console = DedicatedConsole::new();
        console.fail("EIO".to_string());
        assert!(console.drain(&mut |_| {}).is_err());
        console.close();
        assert!(console.drain(&mut |_| {}).is_err());
    }
}
