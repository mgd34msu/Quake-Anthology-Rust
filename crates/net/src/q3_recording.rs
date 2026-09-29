//! Quake III client demo recording.
//!
//! Donor provenance: `Q3DemoSink`, `q3DemoGamestate`, and
//! `Q3DemoRecording` in `src/network/q3/recording.ts` (`CL_Record_f` and
//! `CL_WriteDemoMessage` in `cl_main.c`). Recording builds on the real
//! [`Q3ClientConnection`](crate::q3_net::Q3ClientConnection); the demo
//! file sink is caller-provided, and the filesystem owner chooses and
//! opens the `.dm_68` path.

use crate::q3_net::{encode_server_message, Q3ClientConnection, Q3NetError, ServerMessageContext, ServerOperation};

/// Demo file sink (donor `Q3DemoSink`).
pub trait Q3DemoSink {
    /// Append bytes.
    fn write_bytes(&mut self, bytes: &[u8]);
    /// Close the file.
    fn close(&mut self);
}

/// Gamestate seed (`q3DemoGamestate`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3DemoGamestate {
    /// Demo sequence (`serverMessageSequence - 1`).
    pub sequence: i32,
    /// Encoded gamestate message.
    pub message: Vec<u8>,
}

/// Build the gamestate seed (`q3DemoGamestate`).
pub fn q3_demo_gamestate(client: &Q3ClientConnection<'_>) -> Result<Q3DemoGamestate, Q3NetError> {
    let sequence = client.server_message_sequence.wrapping_sub(1);
    let context = ServerMessageContext {
        product: client.product,
        message_number: sequence,
        reliable_sequence: client.reliable.sequence(),
        server_command_sequence: client.server_command_sequence,
        parse_entities_number: 0,
        baseline: &mut |number| client.baselines.get(number as usize).cloned(),
        history: &mut |_| None,
    };
    let message = encode_server_message(
        client.reliable.sequence(),
        &[ServerOperation::Gamestate(Box::new(client.copy_gamestate()?))],
        &context,
    )?;
    Ok(Q3DemoGamestate { sequence, message })
}

/// Demo recording (`Q3DemoRecording`).
pub struct Q3DemoRecording<'c, 'b, S>
where
    'b: 'c,
{
    client: &'c mut Q3ClientConnection<'b>,
    file: S,
    ended: bool,
}

impl<'c, 'b, S: Q3DemoSink> Q3DemoRecording<'c, 'b, S>
where
    'b: 'c,
{
    /// Start recording over a client and sink.
    pub fn new(client: &'c mut Q3ClientConnection<'b>, mut file: S) -> Result<Self, Q3NetError> {
        client.demo_waiting = true;
        let seed = q3_demo_gamestate(client)?;
        Self::write(&mut file, seed.sequence, &seed.message);
        Ok(Self {
            client,
            file,
            ended: false,
        })
    }

    /// Write a sequenced message with its 8-byte header (`write`).
    fn write(file: &mut S, sequence: i32, bytes: &[u8]) {
        let mut header = [0u8; 8];
        header[0..4].copy_from_slice(&sequence.to_le_bytes());
        header[4..8].copy_from_slice(&(bytes.len() as i32).to_le_bytes());
        file.write_bytes(&header);
        file.write_bytes(bytes);
    }

    /// Borrow the client.
    pub fn client(&self) -> &Q3ClientConnection<'b> {
        self.client
    }

    /// Mutably borrow the client.
    pub fn client_mut(&mut self) -> &mut Q3ClientConnection<'b> {
        self.client
    }

    /// Record a decrypted server payload (`append`).
    ///
    /// Called after parsing each payload with the channel header
    /// removed; payloads only flow once the client leaves the
    /// demo-waiting state.
    pub fn append(&mut self, bytes: &[u8]) -> Result<(), Q3NetError> {
        if self.ended {
            return Err(Q3NetError::Protocol("Q3 demo recording is closed"));
        }
        if !self.client.demo_waiting {
            let sequence = self.client.server_message_sequence;
            Self::write(&mut self.file, sequence, bytes);
        }
        Ok(())
    }

    /// Write the terminator and close (`stop`).
    pub fn stop(&mut self) {
        if self.ended {
            return;
        }
        self.ended = true;
        self.file.write_bytes(&[255u8; 8]);
        self.file.close();
    }

    /// Close without a terminator (`close`).
    pub fn close(&mut self) {
        if !self.ended {
            self.ended = true;
            self.file.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q3_net::{
        DownloadBlock, Gamestate, Q3ClientBindings, Q3ClientMode, Q3ConnectionIdentity, Q3Product, Snapshot,
    };
    use qa_core::identity::IdentityOwner;

    struct Sink {
        bytes: Vec<u8>,
        closes: usize,
    }

    impl Q3DemoSink for Sink {
        fn write_bytes(&mut self, bytes: &[u8]) {
            self.bytes.extend_from_slice(bytes);
        }

        fn close(&mut self) {
            self.closes += 1;
        }
    }

    struct Bindings;

    impl Q3ClientBindings for Bindings {
        fn assert_current(&mut self) {}
        fn print(&mut self, _text: &str) {}
        fn clear_active(&mut self) {}
        fn system_info(&mut self, _info: &str) {}
        fn gamestate(&mut self, _state: &Gamestate, _generation: i32) {}
        fn snapshot(&mut self, _snapshot: &Snapshot, _ping: i32) {}
        fn download_size(&mut self, size: i32) -> i32 {
            size
        }
        fn download(&mut self, _block: &DownloadBlock) {}
        fn map_restart(&mut self) {}
        fn level_shot(&mut self) {}
        fn local_server_running(&self) -> bool {
            false
        }
    }

    #[test]
    fn seed_gamestate_writes_framed_header() {
        let owner = IdentityOwner::create("q3-record").unwrap();
        let mut bindings = Bindings;
        let mut client = Q3ClientConnection::new(
            Q3ConnectionIdentity {
                client: owner.client(0, 0),
                seat: None,
            },
            Q3Product::Base,
            Q3ClientMode::Network {
                challenge: 1,
                qport: 27960,
            },
            &mut bindings,
        );
        client.server_message_sequence = 41;
        let seed = q3_demo_gamestate(&client).unwrap();
        assert_eq!(seed.sequence, 40);
        assert!(!seed.message.is_empty());
        let mut recording = Q3DemoRecording::new(
            &mut client,
            Sink {
                bytes: Vec::new(),
                closes: 0,
            },
        )
        .unwrap();
        assert!(recording.client().demo_waiting);
        // Appends wait for the client to leave the demo-waiting state.
        recording.append(b"early").unwrap();
        recording.client_mut().demo_waiting = false;
        recording.client_mut().server_message_sequence = 42;
        recording.append(b"payload").unwrap();
        recording.stop();
        recording.stop();
        assert_eq!(recording.file.closes, 1);
        let bytes = &recording.file.bytes;
        // Seed frame: sequence 40 + length + gamestate.
        assert_eq!(&bytes[0..4], &40i32.to_le_bytes());
        let seed_length = i32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        assert_eq!(seed_length, seed.message.len());
        assert_eq!(&bytes[8..8 + seed_length], seed.message.as_slice());
        // Payload frame follows with sequence 42.
        let at = 8 + seed_length;
        assert_eq!(&bytes[at..at + 4], &42i32.to_le_bytes());
        assert_eq!(&bytes[at + 4..at + 8], &7i32.to_le_bytes());
        assert_eq!(&bytes[at + 8..at + 15], b"payload");
        // Terminator closes the file.
        assert_eq!(&bytes[at + 15..], &[255u8; 8]);
        // Appends after the end fail.
        assert_eq!(
            recording.append(b"late"),
            Err(Q3NetError::Protocol("Q3 demo recording is closed"))
        );
    }

    #[test]
    fn close_skips_terminator() {
        let owner = IdentityOwner::create("q3-record-close").unwrap();
        let mut bindings = Bindings;
        let mut client = Q3ClientConnection::new(
            Q3ConnectionIdentity {
                client: owner.client(0, 0),
                seat: None,
            },
            Q3Product::Base,
            Q3ClientMode::Network {
                challenge: 1,
                qport: 27960,
            },
            &mut bindings,
        );
        let mut recording = Q3DemoRecording::new(
            &mut client,
            Sink {
                bytes: Vec::new(),
                closes: 0,
            },
        )
        .unwrap();
        let seed_length = recording.file.bytes.len();
        recording.close();
        recording.close();
        assert_eq!(recording.file.closes, 1);
        assert_eq!(recording.file.bytes.len(), seed_length);
    }
}
