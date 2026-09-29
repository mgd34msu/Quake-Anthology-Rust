//! MIDI 1.0 note decoding for the source key range.
//!
//! Donor provenance: `src/input/source-midi.ts`
//! (`SourceMidiDecoder`, from `win32/win_input.c` `MidiInProc`).

use super::KeyCode;

/// MIDI 1.0 byte framing with running status.
#[derive(Debug, Clone, Default)]
pub struct SourceMidiDecoder {
    status: u8,
    first: Option<u8>,
}

impl SourceMidiDecoder {
    /// Fresh decoder.
    #[must_use]
    pub const fn new() -> Self {
        Self { status: 0, first: None }
    }

    /// Drop running status and any partial message.
    pub fn reset(&mut self) {
        self.status = 0;
        self.first = None;
    }

    /// Decode bytes on a 1-based channel, queueing Aux keys.
    ///
    /// `time` is the host message time. Note-on with velocity zero
    /// queues both the release and the press, matching the source.
    pub fn feed(&mut self, bytes: &[u8], channel: i32, time: i64, queue_key: &mut dyn FnMut(i32, bool, i64)) {
        for &byte in bytes {
            if byte >= 0xf8 {
                continue;
            }
            if byte >= 0x80 {
                self.status = if byte < 0xf0 { byte } else { 0 };
                self.first = None;
                continue;
            }
            if self.status == 0 {
                continue;
            }
            let command = self.status & 0xf0;
            if command == 0xc0 || command == 0xd0 {
                continue;
            }
            let Some(note) = self.first.take() else {
                self.first = Some(byte);
                continue;
            };
            if i32::from(self.status & 0x0f) + 1 != channel {
                continue;
            }
            if command != 0x80 && command != 0x90 {
                continue;
            }
            let key = i32::from(note) - 60 + KeyCode::Aux1 as i32;
            if !(KeyCode::Aux1 as i32..=255).contains(&key) {
                continue;
            }
            if command == 0x80 || byte == 0 {
                queue_key(key, false, time);
            }
            if command == 0x90 {
                queue_key(key, true, time);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(bytes: &[u8], channel: i32) -> Vec<(i32, bool)> {
        let mut decoder = SourceMidiDecoder::new();
        let mut keys = Vec::new();
        decoder.feed(bytes, channel, 12, &mut |key, down, time| {
            assert_eq!(time, 12);
            keys.push((key, down));
        });
        keys
    }

    #[test]
    fn decodes_notes_with_running_status() {
        let aux = KeyCode::Aux1 as i32;
        assert_eq!(decode(&[0x90, 60, 100, 62, 100], 1), vec![(aux, true), (aux + 2, true)]);
        assert_eq!(decode(&[0x80, 60, 0], 1), vec![(aux, false)]);
        assert_eq!(decode(&[0x90, 60, 0], 1), vec![(aux, false), (aux, true)]);
        assert!(decode(&[0x90, 60, 100], 2).is_empty());
        assert!(decode(&[0xC0, 5, 60, 100], 1).is_empty());
        assert_eq!(decode(&[0x90, 60, 100, 0xF8, 62, 100], 1), vec![(aux, true), (aux + 2, true)]);
        assert!(decode(&[0x90, 59, 100], 1).is_empty());
        assert!(decode(&[0x90, 60, 100, 0xF0, 62, 100], 1).len() == 1);
        let mut decoder = SourceMidiDecoder::new();
        decoder.feed(&[0x90, 60], 1, 0, &mut |_, _, _| {});
        decoder.reset();
        let mut keys = Vec::new();
        decoder.feed(&[100], 1, 0, &mut |key, down, _| keys.push((key, down)));
        assert!(keys.is_empty());
    }
}
