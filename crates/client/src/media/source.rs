//! Media inputs: ranged reads over bytes or files.
//!
//! Donor provenance: `src/media/source.ts` (`MediaInput`,
//! `mediaBytes`, `openMediaFile`, `readMedia`).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use crate::ClientError;

/// A media input (`MediaInput`).
pub trait MediaInput {
    /// Source name.
    fn source(&self) -> &str;
    /// Byte length.
    fn byte_length(&self) -> usize;
    /// Read at an offset; returns bytes read.
    fn read_at(&mut self, offset: usize, destination: &mut [u8]) -> Result<usize, ClientError>;
}

/// In-memory media (`mediaBytes`).
#[derive(Debug, Clone)]
pub struct MemMedia {
    source: String,
    bytes: Vec<u8>,
    closed: bool,
}

impl MemMedia {
    /// New memory media.
    #[must_use]
    pub fn new(bytes: Vec<u8>, source: &str) -> Self {
        Self {
            source: source.to_string(),
            bytes,
            closed: false,
        }
    }

    /// Close the input.
    pub fn close(&mut self) {
        self.closed = true;
    }
}

impl MediaInput for MemMedia {
    fn source(&self) -> &str {
        &self.source
    }

    fn byte_length(&self) -> usize {
        self.bytes.len()
    }

    fn read_at(&mut self, offset: usize, destination: &mut [u8]) -> Result<usize, ClientError> {
        if self.closed {
            return Err(ClientError::BadMedia("Media input is closed".to_string()));
        }
        let part = self.bytes.get(offset..).unwrap_or(&[]);
        let count = part.len().min(destination.len());
        destination[..count].copy_from_slice(&part[..count]);
        Ok(count)
    }
}

/// File-backed media (`openMediaFile`, one descriptor).
#[derive(Debug)]
pub struct FileMedia {
    source: String,
    byte_length: usize,
    file: Option<File>,
}

impl FileMedia {
    /// Open a media file.
    pub fn open(path: &str) -> Result<Self, ClientError> {
        let file = File::open(path)
            .map_err(|error| ClientError::BadMedia(format!("{path}: {error}")))?;
        let byte_length = file
            .metadata()
            .map_err(|error| ClientError::BadMedia(format!("{path}: {error}")))?
            .len() as usize;
        Ok(Self {
            source: path.to_string(),
            byte_length,
            file: Some(file),
        })
    }

    /// Close the input.
    pub fn close(&mut self) {
        self.file = None;
    }
}

impl MediaInput for FileMedia {
    fn source(&self) -> &str {
        &self.source
    }

    fn byte_length(&self) -> usize {
        self.byte_length
    }

    fn read_at(&mut self, offset: usize, destination: &mut [u8]) -> Result<usize, ClientError> {
        let Some(file) = self.file.as_mut() else {
            return Err(ClientError::BadMedia("Media input is closed".to_string()));
        };
        file.seek(SeekFrom::Start(offset as u64))
            .map_err(|error| ClientError::BadMedia(format!("{}: {error}", self.source)))?;
        let mut count = 0usize;
        while count < destination.len() {
            match file.read(&mut destination[count..]) {
                Ok(0) => break,
                Ok(read) => count += read,
                Err(error) => {
                    return Err(ClientError::BadMedia(format!("{}: {error}", self.source)));
                }
            }
        }
        Ok(count)
    }
}

/// Read an exact range (`readMedia`).
pub fn read_media(
    input: &mut dyn MediaInput,
    offset: usize,
    length: usize,
) -> Result<Vec<u8>, ClientError> {
    if offset > input.byte_length().saturating_sub(length) {
        return Err(ClientError::BadMedia(format!(
            "{}:{offset}: truncated media read of {length} bytes",
            input.source()
        )));
    }
    let mut bytes = vec![0u8; length];
    let mut count = 0usize;
    while count < length {
        let read = input.read_at(offset + count, &mut bytes[count..])?;
        if read == 0 || read > length - count {
            return Err(ClientError::BadMedia(format!(
                "{}:{}: short media read",
                input.source(),
                offset + count
            )));
        }
        count += read;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ranges() {
        let mut input = MemMedia::new(vec![1, 2, 3, 4], "<test>");
        assert_eq!(read_media(&mut input, 1, 2).unwrap(), vec![2, 3]);
        assert!(read_media(&mut input, 3, 2).is_err());
    }

    #[test]
    fn closed_is_an_error() {
        let mut input = MemMedia::new(vec![1], "<test>");
        input.close();
        assert!(read_media(&mut input, 0, 1).is_err());
    }
}
