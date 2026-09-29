//! Quake III netchan fragment ordering ported from
//! `src/network/common/fragments.ts`.

use thiserror::Error;

/// Error for invalid fragment sizes.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FragmentError {
    /// Fragment or message sizes are invalid.
    #[error("Invalid fragment sizes")]
    BadSizes,
    /// Fragmented message is still pending.
    #[error("Fragmented message is still pending")]
    StillPending,
    /// Message exceeds fragment capacity.
    #[error("Message exceeds fragment capacity")]
    TooLarge,
    /// Receive capacity is invalid.
    #[error("Invalid fragment receive capacity")]
    BadCapacity,
}

/// One message fragment (`MessageFragment`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageFragment {
    /// Message sequence.
    pub sequence: i32,
    /// Byte offset of this fragment.
    pub offset: usize,
    /// Fragment bytes.
    pub bytes: Vec<u8>,
    /// True for the final fragment.
    pub final_fragment: bool,
}

/// Fragment receive result (`FragmentResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FragmentResult {
    /// More fragments pending.
    Pending {
        /// Bytes accepted so far.
        byte_length: usize,
    },
    /// Message complete.
    Complete {
        /// Message sequence.
        sequence: i32,
        /// Reassembled bytes.
        bytes: Vec<u8>,
    },
    /// Fragment rejected.
    Rejected {
        /// Rejection reason.
        reason: FragmentRejection,
    },
}

/// Fragment rejection reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FragmentRejection {
    /// Stale sequence.
    Sequence,
    /// Out-of-order offset.
    Order,
    /// Message exceeds capacity.
    Length,
}

/// Fragmenting sender (`FragmentSender`).
#[derive(Debug)]
pub struct FragmentSender {
    fragment_bytes: usize,
    max_message_bytes: usize,
    terminal_empty_fragment: bool,
    bytes: Option<Vec<u8>>,
    offset: usize,
    sequence: i32,
}

impl FragmentSender {
    /// Create a sender with fragment, message, and terminal-fragment policy.
    pub fn new(
        fragment_bytes: usize,
        max_message_bytes: usize,
        terminal_empty_fragment: bool,
    ) -> Result<Self, FragmentError> {
        if fragment_bytes == 0 || max_message_bytes < fragment_bytes {
            return Err(FragmentError::BadSizes);
        }
        Ok(Self {
            fragment_bytes,
            max_message_bytes,
            terminal_empty_fragment,
            bytes: None,
            offset: 0,
            sequence: 0,
        })
    }

    /// True while a message is being emitted.
    #[must_use]
    pub fn pending(&self) -> bool {
        self.bytes.is_some()
    }

    /// Begin fragmenting `bytes` under `sequence`.
    pub fn begin(&mut self, sequence: i32, bytes: &[u8]) -> Result<(), FragmentError> {
        if self.pending() {
            return Err(FragmentError::StillPending);
        }
        if bytes.len() > self.max_message_bytes {
            return Err(FragmentError::TooLarge);
        }
        self.sequence = sequence;
        self.bytes = Some(bytes.to_vec());
        self.offset = 0;
        Ok(())
    }
}

impl Iterator for FragmentSender {
    type Item = MessageFragment;

    /// Next fragment, or `None` when the message is complete.
    fn next(&mut self) -> Option<MessageFragment> {
        let bytes = self.bytes.as_ref()?;
        let offset = self.offset;
        let end = bytes.len().min(offset + self.fragment_bytes);
        let fragment = bytes[offset..end].to_vec();
        self.offset = end;
        let final_fragment =
            end == bytes.len() && (!self.terminal_empty_fragment || fragment.len() < self.fragment_bytes);
        if final_fragment {
            self.bytes = None;
        }
        Some(MessageFragment {
            sequence: self.sequence,
            offset,
            bytes: fragment,
            final_fragment,
        })
    }
}

/// Reassembling receiver (`FragmentReceiver`).
#[derive(Debug)]
pub struct FragmentReceiver {
    max_message_bytes: usize,
    current: i32,
    accepted: i32,
    length: usize,
    storage: Vec<u8>,
}

impl FragmentReceiver {
    /// Create a receiver with `max_message_bytes` capacity.
    pub fn new(max_message_bytes: usize) -> Result<Self, FragmentError> {
        if max_message_bytes == 0 {
            return Err(FragmentError::BadCapacity);
        }
        Ok(Self {
            max_message_bytes,
            current: -1,
            accepted: 0,
            length: 0,
            storage: vec![0; max_message_bytes],
        })
    }

    /// Receive a fragment (`receive`).
    pub fn receive(&mut self, fragment: &MessageFragment) -> FragmentResult {
        if fragment.sequence <= self.accepted {
            return FragmentResult::Rejected {
                reason: FragmentRejection::Sequence,
            };
        }
        if fragment.sequence != self.current {
            self.current = fragment.sequence;
            self.length = 0;
        }
        if fragment.offset != self.length {
            return FragmentResult::Rejected {
                reason: FragmentRejection::Order,
            };
        }
        if self.length + fragment.bytes.len() > self.max_message_bytes {
            return FragmentResult::Rejected {
                reason: FragmentRejection::Length,
            };
        }
        self.storage[self.length..self.length + fragment.bytes.len()].copy_from_slice(&fragment.bytes);
        self.length += fragment.bytes.len();
        if !fragment.final_fragment {
            return FragmentResult::Pending {
                byte_length: self.length,
            };
        }
        let bytes = self.storage[..self.length].to_vec();
        self.length = 0;
        self.accepted = fragment.sequence;
        FragmentResult::Complete {
            sequence: fragment.sequence,
            bytes,
        }
    }

    /// Accept an unfragmented sequence (`acceptUnfragmented`).
    pub fn accept_unfragmented(&mut self, sequence: i32) -> bool {
        if sequence <= self.accepted {
            return false;
        }
        self.accepted = sequence;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fragments_reassemble_in_order() {
        let mut sender = FragmentSender::new(4, 16, false).unwrap();
        let mut receiver = FragmentReceiver::new(16).unwrap();
        sender.begin(7, &[1, 2, 3, 4, 5, 6]).unwrap();
        let mut result = FragmentResult::Pending { byte_length: 0 };
        while sender.pending() {
            let fragment = sender.next().unwrap();
            result = receiver.receive(&fragment);
        }
        assert_eq!(
            result,
            FragmentResult::Complete {
                sequence: 7,
                bytes: vec![1, 2, 3, 4, 5, 6],
            }
        );
        assert!(matches!(
            receiver.receive(&MessageFragment {
                sequence: 7,
                offset: 0,
                bytes: vec![1],
                final_fragment: true,
            }),
            FragmentResult::Rejected { .. }
        ));
    }
}
