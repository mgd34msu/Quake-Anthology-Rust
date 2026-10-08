//! Bounded nonblocking terminal/pipe lines feeding the one system-event ring.
use crate::clock::Clock;
use qa_core::sys_events::{EventKind, SysEvent, SysEventQueue};
use std::{
    fs::File,
    io::{self, Read},
};

const LINE_BYTES: usize = 8191;
pub(crate) struct ConsoleInput {
    file: Option<File>,
    line: Box<[u8; LINE_BYTES]>,
    len: usize,
    ready: bool,
    discard: bool,
    skip_lf: bool,
    pending: [u8; 4096],
    cursor: usize,
    end: usize,
    eof: bool,
    pub lines: u64,
    pub discarded: u64,
    pub errors: u64,
}
impl ConsoleInput {
    pub fn open() -> Self {
        #[cfg(target_os = "linux")]
        let file = {
            use std::{
                io::Seek,
                os::{fd::BorrowedFd, unix::fs::OpenOptionsExt},
            };
            // A fresh open file description keeps inherited stdin flags intact.
            // O_NONBLOCK is 0x800 on the supported Linux x86_64/aarch64 hosts.
            let mut file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(0x800)
                .open("/proc/self/fd/0")
                .ok();
            if let Some(reopened) = &mut file {
                // SAFETY: the successful /proc open establishes fd 0 exists;
                // platform owns stdin and the clone is closed independently.
                let inherited = unsafe { BorrowedFd::borrow_raw(0) }.try_clone_to_owned();
                if let Ok(inherited) = inherited {
                    let mut inherited = File::from(inherited);
                    if let Ok(position) = inherited.stream_position() {
                        let _ = reopened.seek(std::io::SeekFrom::Start(position));
                    }
                }
            }
            file
        };
        #[cfg(not(target_os = "linux"))]
        let file = None;
        Self {
            file,
            line: Box::new([0; LINE_BYTES]),
            len: 0,
            ready: false,
            discard: false,
            skip_lf: false,
            pending: [0; 4096],
            cursor: 0,
            end: 0,
            eof: false,
            lines: 0,
            discarded: 0,
            errors: 0,
        }
    }
    pub fn poll(&mut self, clock: &Clock, queue: &mut SysEventQueue) {
        // A continuous producer cannot starve SDL, network or the final Time.
        let mut emitted = 0;
        let mut consumed = 0;
        while emitted < 8 && consumed < 4096 && queue.len() + 1 < queue.capacity() {
            if self.ready {
                if self.len > 0 {
                    if let Ok(text) = std::str::from_utf8(&self.line[..self.len]) {
                        if queue
                            .push(SysEvent {
                                time: clock.now(),
                                kind: EventKind::ConsoleLine(text),
                            })
                            .is_err()
                        {
                            break;
                        }
                        self.lines += 1;
                        emitted += 1;
                    } else {
                        self.discarded += 1;
                    }
                }
                self.ready = false;
                self.len = 0;
            }
            if self.cursor == self.end {
                if self.eof {
                    break;
                }
                let Some(file) = self.file.as_mut() else {
                    break;
                };
                match file.read(&mut self.pending) {
                    Ok(0) => {
                        self.eof = true;
                        self.file = None;
                        if self.discard {
                            self.discard = false;
                            self.discarded += 1;
                            self.len = 0;
                        } else {
                            self.ready = self.len > 0;
                        }
                        continue;
                    }
                    Ok(count) => {
                        self.cursor = 0;
                        self.end = count;
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) =>
                    {
                        break;
                    }
                    Err(_) => {
                        self.errors += 1;
                        self.file = None;
                        break;
                    }
                }
            }
            let byte = self.pending[self.cursor];
            self.cursor += 1;
            consumed += 1;
            if self.skip_lf {
                self.skip_lf = false;
                if byte == b'\n' {
                    continue;
                }
            }
            if matches!(byte, b'\r' | b'\n') {
                self.skip_lf = byte == b'\r';
                if self.discard {
                    self.discard = false;
                    self.len = 0;
                    self.discarded += 1;
                } else {
                    self.ready = true;
                }
            } else if !self.discard {
                if self.len == LINE_BYTES || byte == 0 {
                    self.discard = true;
                    self.len = 0;
                } else {
                    self.line[self.len] = byte;
                    self.len += 1;
                }
            }
        }
    }
}
