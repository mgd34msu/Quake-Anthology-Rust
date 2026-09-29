//! Linux ALSA raw-MIDI source input.
//!
//! Donor provenance: `src/input/midi.ts` (`LinuxMidiInputBoundary`,
//! `SourceMidiInput`, from `win32/win_input.c` `IN_StartupMIDI`).

use std::collections::HashSet;

use qa_core::cvar::{CvarError, CvarRegistry};
use thiserror::Error;

use super::device::register_midi_settings;
use super::source_midi::SourceMidiDecoder;

/// MIDI error.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MidiError {
    /// MIDI input currently requires Linux ALSA raw MIDI.
    #[error("MIDI input currently requires Linux ALSA raw MIDI")]
    NotLinux,
    /// ALSA MIDI device path is invalid.
    #[error("Invalid ALSA MIDI device path")]
    BadPath,
    /// ALSA MIDI path is not a character device.
    #[error("ALSA MIDI path is not a character device")]
    NotCharacterDevice,
    /// MIDI read requires a nonempty buffer.
    #[error("MIDI read requires a nonempty buffer")]
    EmptyBuffer,
    /// Invalid MIDI read length.
    #[error("Invalid MIDI read length")]
    BadReadLength,
    /// MIDI input reached end of stream.
    #[error("MIDI input reached end of stream")]
    EndOfStream,
    /// MIDI input handle is closed.
    #[error("MIDI input handle is closed")]
    ClosedHandle,
    /// Missing initial MIDI input byte.
    #[error("Missing initial MIDI input byte")]
    MissingInitialByte,
    /// Missing MIDI device coordinates.
    #[error("Missing MIDI device coordinates")]
    MissingCoordinates,
    /// Device index is outside the available devices.
    #[error("MIDI device index is outside {0} available devices")]
    BadDeviceIndex(usize),
    /// Invalid MIDI input boundary read length.
    #[error("Invalid MIDI input boundary read length")]
    BadBoundaryLength,
    /// Source MIDI input is closed.
    #[error("Source MIDI input is closed")]
    Closed,
    /// A MIDI cvar no longer exists.
    #[error("MIDI cvar {0} no longer exists")]
    MissingCvar(String),
    /// Filesystem error.
    #[error("MIDI file error: {0}")]
    File(String),
    /// Cvar error.
    #[error(transparent)]
    Cvar(#[from] CvarError),
}

/// One ALSA raw-MIDI device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidiDevice {
    /// Display name.
    pub name: String,
    /// System path.
    pub path: String,
}

/// Open MIDI input stream.
pub trait MidiInputHandle {
    /// Read bytes; zero means no input is ready.
    fn read(&mut self, bytes: &mut [u8]) -> Result<usize, MidiError>;
    /// Close the stream.
    fn close(&mut self);
}

/// Device enumeration and opening.
pub trait MidiInputBoundary {
    /// List hardware MIDI 1.0 nodes.
    fn list(&self) -> Result<Vec<MidiDevice>, MidiError>;
    /// Open one device for input.
    fn open(&self, device: &MidiDevice) -> Result<Box<dyn MidiInputHandle>, MidiError>;
}

/// Raw file operations behind [`LinuxMidiInputBoundary`].
pub trait MidiFileIo {
    /// List `/dev/snd` entries.
    fn read_directory(&self) -> Result<Vec<String>, MidiError>;
    /// Open a path; returns a handle id.
    fn open(&self, path: &str) -> Result<u64, MidiError>;
    /// Whether a handle is a character device.
    fn is_character_device(&self, handle: u64) -> bool;
    /// Nonblocking read; zero means no input is ready.
    fn read(&self, handle: u64, bytes: &mut [u8]) -> Result<usize, MidiError>;
    /// Close a handle.
    fn close(&self, handle: u64);
}

fn device_coordinates(name: &str) -> Option<(u64, u64)> {
    let rest = name.strip_prefix("midiC")?;
    let (card_text, device_text) = rest.split_once('D')?;
    if card_text.is_empty() || device_text.is_empty() {
        return None;
    }
    if !card_text.chars().all(|value| value.is_ascii_digit()) || !device_text.chars().all(|value| value.is_ascii_digit()) {
        return None;
    }
    Some((card_text.parse().ok()?, device_text.parse().ok()?))
}

fn valid_device_path(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("/dev/snd/midiC") else {
        return false;
    };
    let Some((card, device)) = rest.split_once('D') else {
        return false;
    };
    !card.is_empty() && !device.is_empty() && card.chars().all(|value| value.is_ascii_digit()) && device.chars().all(|value| value.is_ascii_digit())
}

struct RawMidiHandle<IO> {
    handle: u64,
    io: std::rc::Rc<IO>,
    closed: bool,
    pending: Option<u8>,
}

impl<IO: MidiFileIo> RawMidiHandle<IO> {
    fn start(&mut self) -> Result<(), MidiError> {
        let mut first = [0u8; 1];
        if self.read(&mut first)? == 0 {
            return Ok(());
        }
        self.pending = Some(first[0]);
        Ok(())
    }
}

impl<IO: MidiFileIo> MidiInputHandle for RawMidiHandle<IO> {
    fn read(&mut self, bytes: &mut [u8]) -> Result<usize, MidiError> {
        if self.closed {
            return Err(MidiError::ClosedHandle);
        }
        if bytes.is_empty() {
            return Err(MidiError::EmptyBuffer);
        }
        if let Some(pending) = self.pending.take() {
            bytes[0] = pending;
            return Ok(1);
        }
        let count = self.io.read(self.handle, bytes)?;
        if count > bytes.len() {
            return Err(MidiError::BadReadLength);
        }
        Ok(count)
    }

    fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.io.close(self.handle);
    }
}

/// ALSA raw-MIDI boundary over injected file operations.
pub struct LinuxMidiInputBoundary<IO: MidiFileIo> {
    io: std::rc::Rc<IO>,
}

impl<IO: MidiFileIo> LinuxMidiInputBoundary<IO> {
    /// Boundary over file operations.
    #[must_use]
    pub fn new(io: IO) -> Self {
        Self {
            io: std::rc::Rc::new(io),
        }
    }
}

impl<IO: MidiFileIo + 'static> MidiInputBoundary for LinuxMidiInputBoundary<IO> {
    fn list(&self) -> Result<Vec<MidiDevice>, MidiError> {
        if !cfg!(target_os = "linux") {
            return Err(MidiError::NotLinux);
        }
        let mut devices: Vec<(u64, u64, String)> = Vec::new();
        for name in self.io.read_directory()? {
            let Some((card, device)) = device_coordinates(&name) else {
                continue;
            };
            devices.push((card, device, name));
        }
        devices.sort();
        Ok(devices
            .iter()
            .map(|(_, _, name)| MidiDevice {
                name: format!("ALSA {name}"),
                path: format!("/dev/snd/{name}"),
            })
            .collect())
    }

    fn open(&self, device: &MidiDevice) -> Result<Box<dyn MidiInputHandle>, MidiError> {
        if !cfg!(target_os = "linux") {
            return Err(MidiError::NotLinux);
        }
        if !valid_device_path(&device.path) {
            return Err(MidiError::BadPath);
        }
        // O_RDONLY never opens a MIDI output substream; O_NONBLOCK
        // applies to both open and read, and read starts capture.
        let handle = self.io.open(&device.path)?;
        if !self.io.is_character_device(handle) {
            self.io.close(handle);
            return Err(MidiError::NotCharacterDevice);
        }
        let mut stream = RawMidiHandle {
            handle,
            io: self.io.clone(),
            closed: false,
            pending: None,
        };
        if let Err(error) = stream.start() {
            self.io.close(handle);
            return Err(error);
        }
        Ok(Box::new(stream))
    }
}

#[cfg(target_os = "linux")]
mod std_io {
    use super::{MidiError, MidiFileIo};
    use std::collections::HashMap;
    use std::fs::{File, OpenOptions};
    use std::io::Read;
    use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    const O_NONBLOCK: i32 = 0o2000;
    const O_NOFOLLOW: i32 = 0o400000;

    /// [`MidiFileIo`] over the live `/dev/snd` tree.
    pub struct StdMidiFileIo {
        next: AtomicU64,
        files: Mutex<HashMap<u64, File>>,
    }

    impl StdMidiFileIo {
        /// Live file operations.
        #[must_use]
        pub fn new() -> Self {
            Self {
                next: AtomicU64::new(1),
                files: Mutex::new(HashMap::new()),
            }
        }
    }

    impl Default for StdMidiFileIo {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MidiFileIo for StdMidiFileIo {
        fn read_directory(&self) -> Result<Vec<String>, MidiError> {
            match std::fs::read_dir("/dev/snd") {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
                Err(error) => Err(MidiError::File(error.to_string())),
                Ok(entries) => {
                    let mut names = Vec::new();
                    for entry in entries {
                        let entry = entry.map_err(|error| MidiError::File(error.to_string()))?;
                        names.push(entry.file_name().to_string_lossy().into_owned());
                    }
                    Ok(names)
                }
            }
        }

        fn open(&self, path: &str) -> Result<u64, MidiError> {
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(O_NONBLOCK | O_NOFOLLOW)
                .open(path)
                .map_err(|error| MidiError::File(error.to_string()))?;
            let handle = self.next.fetch_add(1, Ordering::Relaxed);
            self.files.lock().expect("midi handle lock").insert(handle, file);
            Ok(handle)
        }

        fn is_character_device(&self, handle: u64) -> bool {
            self.files
                .lock()
                .expect("midi handle lock")
                .get(&handle)
                .and_then(|file| file.metadata().ok())
                .is_some_and(|metadata| metadata.file_type().is_char_device())
        }

        fn read(&self, handle: u64, bytes: &mut [u8]) -> Result<usize, MidiError> {
            let mut files = self.files.lock().expect("midi handle lock");
            let Some(file) = files.get_mut(&handle) else {
                return Err(MidiError::ClosedHandle);
            };
            match file.read(bytes) {
                Ok(0) => Err(MidiError::EndOfStream),
                Ok(count) => Ok(count),
                Err(error) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted) => Ok(0),
                Err(error) => Err(MidiError::File(error.to_string())),
            }
        }

        fn close(&self, handle: u64) {
            self.files.lock().expect("midi handle lock").remove(&handle);
        }
    }
}

#[cfg(target_os = "linux")]
pub use std_io::StdMidiFileIo;

/// Source MIDI input: device lifecycle plus per-frame key queueing.
pub struct SourceMidiInput {
    boundary: Box<dyn MidiInputBoundary>,
    print: Box<dyn FnMut(&str)>,
    decoder: SourceMidiDecoder,
    devices: Vec<MidiDevice>,
    handle: Option<Box<dyn MidiInputHandle>>,
    closed: bool,
    held: HashSet<i32>,
    released: HashSet<i32>,
    channel: Option<i32>,
}

impl SourceMidiInput {
    /// Construction performs no device access, including enumeration.
    #[must_use]
    pub fn new(boundary: Box<dyn MidiInputBoundary>, print: Box<dyn FnMut(&str)>) -> Self {
        Self {
            boundary,
            print,
            decoder: SourceMidiDecoder::new(),
            devices: Vec::new(),
            handle: None,
            closed: false,
            held: HashSet::new(),
            released: HashSet::new(),
            channel: None,
        }
    }

    /// Whether a device is open.
    #[must_use]
    pub const fn connected(&self) -> bool {
        self.handle.is_some()
    }

    /// Enumerate devices.
    pub fn available_devices(&self) -> Result<Vec<MidiDevice>, MidiError> {
        self.boundary.list()
    }

    fn cvar(cvars: &CvarRegistry, name: &str) -> Result<qa_core::cvar::CvarSnapshot, MidiError> {
        cvars.get(name).ok_or_else(|| MidiError::MissingCvar(name.to_string()))
    }

    /// Register settings and open the selected device.
    pub fn initialize(&mut self, cvars: &mut CvarRegistry) -> Result<(), MidiError> {
        if self.closed {
            return Err(MidiError::Closed);
        }
        register_midi_settings(cvars)?;
        self.stop();
        if Self::cvar(cvars, "in_midi")?.numeric_value == 0.0 {
            return Ok(());
        }
        let selected = Self::cvar(cvars, "in_mididevice")?.integer_value;
        match self.boundary.list() {
            Ok(devices) => {
                let device = devices.get(usize::try_from(selected).unwrap_or(usize::MAX)).cloned();
                match device {
                    None => (self.print)(&format!("WARNING: could not open MIDI device {selected}: {}\n", MidiError::BadDeviceIndex(devices.len()))),
                    Some(device) => match self.boundary.open(&device) {
                        Ok(handle) => {
                            self.devices = devices;
                            self.handle = Some(handle);
                        }
                        Err(error) => (self.print)(&format!("WARNING: could not open MIDI device {selected}: {error}\n")),
                    },
                }
            }
            Err(error) => (self.print)(&format!("WARNING: could not open MIDI device {selected}: {error}\n")),
        }
        Ok(())
    }

    /// Re-run initialization.
    pub fn restart(&mut self, cvars: &mut CvarRegistry) -> Result<(), MidiError> {
        self.initialize(cvars)
    }

    /// Pump at most sixteen reads into queued keys.
    pub fn frame(&mut self, cvars: &CvarRegistry, queue_key: &mut dyn FnMut(i32, bool, i64), time: i64) -> Result<(), MidiError> {
        if self.closed {
            return Err(MidiError::Closed);
        }
        let channel = Self::cvar(cvars, "in_midichannel")?.integer_value;
        if Some(channel) != self.channel {
            self.release(time, queue_key);
            self.channel = Some(channel);
        }
        for key in std::mem::take(&mut self.released) {
            queue_key(key, false, time);
        }
        for _ in 0..16 {
            let Some(handle) = self.handle.as_mut() else {
                return Ok(());
            };
            let mut buffer = [0u8; 4096];
            let count = match handle.read(&mut buffer) {
                Ok(count) => count,
                Err(error) => {
                    self.stop();
                    self.release(time, queue_key);
                    (self.print)(&format!("WARNING: MIDI input stopped: {error}\n"));
                    return Ok(());
                }
            };
            // RawMidiHandle yields 1..=len or an error; anything else is
            // a boundary violation, except idle zero which ends the frame.
            if count > buffer.len() {
                return Err(MidiError::BadBoundaryLength);
            }
            if count == 0 {
                return Ok(());
            }
            let held = &mut self.held;
            self.decoder.feed(&buffer[..count], channel, time, &mut |key, down, timestamp| {
                if down {
                    held.insert(key);
                } else {
                    held.remove(&key);
                }
                queue_key(key, down, timestamp);
            });
        }
        Ok(())
    }

    /// Print `MidiInfo_f` status.
    pub fn info(&mut self, cvars: &CvarRegistry) -> Result<(), MidiError> {
        if self.closed {
            return Err(MidiError::Closed);
        }
        (self.print)(&format!(
            "\nMIDI control:       {}\n",
            if Self::cvar(cvars, "in_midi")?.integer_value != 0 { "enabled" } else { "disabled" }
        ));
        (self.print)(&format!("port:               {}\n", Self::cvar(cvars, "in_midiport")?.integer_value));
        (self.print)(&format!("channel:            {}\n", Self::cvar(cvars, "in_midichannel")?.integer_value));
        (self.print)(&format!("current device:     {}\n", Self::cvar(cvars, "in_mididevice")?.integer_value));
        (self.print)(&format!("number of devices:  {}\n", self.devices.len()));
        for (index, device) in self.devices.iter().enumerate() {
            (self.print)(&format!(
                "{}device {:>2}:       {}\n",
                if index as i32 == Self::cvar(cvars, "in_mididevice")?.integer_value { "***" } else { "..." },
                index,
                device.name
            ));
            (self.print)(&format!(
                "...system path:     {}\n...manufacturer/product IDs: unavailable through Linux raw MIDI\n\n",
                device.path
            ));
        }
        Ok(())
    }

    /// Release every held key.
    pub fn release(&mut self, time: i64, queue_key: &mut dyn FnMut(i32, bool, i64)) {
        for key in self.held.iter().chain(self.released.iter()).copied().collect::<Vec<_>>() {
            queue_key(key, false, time);
        }
        self.held.clear();
        self.released.clear();
        self.decoder.reset();
    }

    fn stop(&mut self) {
        for key in std::mem::take(&mut self.held) {
            self.released.insert(key);
        }
        if let Some(mut handle) = self.handle.take() {
            handle.close();
        }
        self.devices.clear();
        self.decoder.reset();
    }

    /// Close MIDI input.
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd::Dialect;
    use std::collections::HashMap;

    struct FakeIo {
        names: Vec<String>,
        streams: std::cell::RefCell<HashMap<u64, Vec<u8>>>,
        next: std::cell::Cell<u64>,
    }

    impl MidiFileIo for FakeIo {
        fn read_directory(&self) -> Result<Vec<String>, MidiError> {
            Ok(self.names.clone())
        }

        fn open(&self, path: &str) -> Result<u64, MidiError> {
            assert!(valid_device_path(path));
            let handle = self.next.get() + 1;
            self.next.set(handle);
            self.streams.borrow_mut().insert(handle, vec![0x90, 60, 100]);
            Ok(handle)
        }

        fn is_character_device(&self, _handle: u64) -> bool {
            true
        }

        fn read(&self, handle: u64, bytes: &mut [u8]) -> Result<usize, MidiError> {
            let mut streams = self.streams.borrow_mut();
            let stream = streams.get_mut(&handle).ok_or(MidiError::ClosedHandle)?;
            if stream.is_empty() {
                return Ok(0);
            }
            let count = stream.len().min(bytes.len());
            bytes[..count].copy_from_slice(&stream[..count]);
            stream.drain(..count);
            Ok(count)
        }

        fn close(&self, handle: u64) {
            self.streams.borrow_mut().remove(&handle);
        }
    }

    #[test]
    fn enumerates_and_opens_sorted_devices() {
        let io = FakeIo {
            names: vec!["midiC1D0".to_string(), "pcmC0D0p".to_string(), "midiC0D2".to_string()],
            streams: std::cell::RefCell::new(HashMap::new()),
            next: std::cell::Cell::new(0),
        };
        let boundary = LinuxMidiInputBoundary::new(io);
        if !cfg!(target_os = "linux") {
            assert_eq!(boundary.list(), Err(MidiError::NotLinux));
            return;
        }
        let devices = boundary.list().unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].path, "/dev/snd/midiC0D2");
        let mut handle = boundary.open(&devices[0]).unwrap();
        let mut bytes = [0u8; 8];
        assert_eq!(handle.read(&mut bytes).unwrap(), 1);
        assert_eq!(bytes[0], 0x90);
        assert!(boundary.open(&MidiDevice { name: String::new(), path: "/tmp/nope".to_string() }).is_err());
    }

    #[test]
    fn source_input_pumps_note_keys() {
        if !cfg!(target_os = "linux") {
            return;
        }
        let io = FakeIo {
            names: vec!["midiC0D0".to_string()],
            streams: std::cell::RefCell::new(HashMap::new()),
            next: std::cell::Cell::new(0),
        };
        let printed = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let sink = printed.clone();
        let mut midi = SourceMidiInput::new(Box::new(LinuxMidiInputBoundary::new(io)), Box::new(move |text| sink.borrow_mut().push(text.to_string())));
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        register_midi_settings(&mut cvars).unwrap();
        cvars.set("in_midi", "1", false).unwrap();
        midi.initialize(&mut cvars).unwrap();
        assert!(midi.connected());
        let mut keys = Vec::new();
        midi.frame(&cvars, &mut |key, down, time| keys.push((key, down, time)), 40).unwrap();
        assert_eq!(keys, vec![(super::super::KeyCode::Aux1 as i32, true, 40)]);
        cvars.set("in_midichannel", "2", false).unwrap();
        midi.frame(&cvars, &mut |key, down, _| keys.push((key, down, 0)), 50).unwrap();
        assert!(keys.iter().any(|(key, down, _)| *key == super::super::KeyCode::Aux1 as i32 && !down));
        midi.info(&cvars).unwrap();
        assert!(printed.borrow().iter().any(|line| line.contains("MIDI control")));
        midi.close();
        assert!(midi.frame(&cvars, &mut |_, _, _| {}, 0).is_err());
    }
}
