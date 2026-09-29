//! Vorbis audio decode over `libvorbisfile`.
//!
//! Donor provenance: `src/platform/vorbis.ts` (Q1/Q2 `libvorbisfile`
//! decoding) and the `VorbisPcmStream` surface from
//! `src/audio/streams.ts`.
//!
//! Only the qualified 64-bit LP64 ABI is supported. Archive bytes
//! decode through an owned temporary file, since the platform ABI
//! owns file decoding.

use std::os::raw::{c_char, c_int, c_long};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::ClientError;

/// `OggVorbis_File` storage: the public 1.x ABI is 944 bytes; retain
/// the source ports' 2048-byte reserve with 8-byte alignment.
#[repr(C, align(8))]
struct VorbisStorage {
    words: [u64; 256],
}

/// `vorbis_info` prefix (`version`, `channels`, `rate`).
#[repr(C)]
struct VorbisInfo {
    version: c_int,
    channels: c_int,
    rate: c_long,
}

#[link(name = "vorbisfile")]
extern "C" {
    fn ov_fopen(path: *const c_char, storage: *mut VorbisStorage) -> c_int;
    fn ov_read(
        storage: *mut VorbisStorage,
        buffer: *mut c_char,
        length: c_int,
        bigendian: c_int,
        word: c_int,
        signed: c_int,
        bitstream: *mut c_int,
    ) -> c_long;
    fn ov_info(storage: *mut VorbisStorage, link: c_int) -> *const VorbisInfo;
    fn ov_streams(storage: *mut VorbisStorage) -> c_long;
    fn ov_pcm_total(storage: *mut VorbisStorage, link: c_int) -> i64;
    fn ov_pcm_tell(storage: *mut VorbisStorage) -> i64;
    fn ov_pcm_seek(storage: *mut VorbisStorage, pos: i64) -> c_int;
    fn ov_clear(storage: *mut VorbisStorage) -> c_int;
}

/// A Vorbis operation failure (`VorbisError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VorbisError {
    /// Operation name.
    pub operation: String,
    /// Native error code.
    pub code: i64,
}

impl std::fmt::Display for VorbisError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} failed with Vorbis error {}", self.operation, self.code)
    }
}

impl std::error::Error for VorbisError {}

/// Vorbis stream metadata (`VorbisMetadata`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VorbisMetadata {
    /// Sample rate.
    pub sample_rate: u32,
    /// Channels.
    pub channels: u8,
    /// Total frames.
    pub total_frames: usize,
    /// Duration in seconds.
    pub duration_seconds: f64,
    /// Logical streams.
    pub logical_streams: usize,
}

/// Decoded PCM samples (`VorbisPcmChunk`; the samples belong to the
/// caller and survive reads, seeks and close).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VorbisPcmChunk {
    /// Samples.
    pub samples: Vec<i16>,
    /// Frames.
    pub frames: usize,
    /// Sample rate.
    pub sample_rate: u32,
    /// Channels.
    pub channels: u8,
}

fn vorbis_error(operation: &str, code: i64) -> ClientError {
    ClientError::BadMedia(
        VorbisError {
            operation: operation.to_string(),
            code,
        }
        .to_string(),
    )
}

fn safe_count(value: i64, name: &str) -> Result<usize, ClientError> {
    if value < 0 {
        return Err(vorbis_error(name, value));
    }
    Ok(value as usize)
}

fn stream_format(storage: *mut VorbisStorage, link: usize) -> Result<(u32, u8), ClientError> {
    // SAFETY: the storage is open across the synchronous call.
    let info = unsafe { ov_info(storage, link as c_int) };
    if info.is_null() {
        return Err(ClientError::BadMedia("ov_info returned no stream metadata".to_string()));
    }
    // SAFETY: `ov_info` returns a live `vorbis_info` prefix.
    let (channels, rate) = unsafe { ((*info).channels, (*info).rate) };
    if channels != 1 && channels != 2 {
        return Err(ClientError::BadMedia(
            "Vorbis output must be mono or stereo".to_string(),
        ));
    }
    if !(8000..=192000).contains(&rate) {
        return Err(ClientError::BadMedia(
            "Vorbis sample rate must be in 8000..192000 Hz".to_string(),
        ));
    }
    Ok((rate as u32, channels as u8))
}

/// A Vorbis decoder (`VorbisDecoder`).
pub struct VorbisDecoder {
    storage: Option<Box<VorbisStorage>>,
    metadata: VorbisMetadata,
}

impl Drop for VorbisDecoder {
    fn drop(&mut self) {
        if let Some(mut storage) = self.storage.take() {
            // SAFETY: the storage is open and cleared once; the
            // result cannot propagate from `Drop`.
            unsafe {
                ov_clear(&mut *storage);
            }
        }
    }
}

impl VorbisDecoder {
    /// Open a Vorbis file (`VorbisDecoder.open`).
    pub fn open(path: &str) -> Result<Self, ClientError> {
        if path.is_empty() || path.contains('\0') {
            return Err(ClientError::BadMedia(
                "Vorbis path must be nonempty and contain no NUL".to_string(),
            ));
        }
        let mut storage = Box::new(VorbisStorage { words: [0; 256] });
        let path = std::ffi::CString::new(path)
            .map_err(|_| ClientError::BadMedia("Vorbis path must be nonempty and contain no NUL".to_string()))?;
        // SAFETY: the path is NUL-terminated and the 8-aligned
        // storage stays alive across the call.
        let result = unsafe { ov_fopen(path.as_ptr(), &mut *storage) };
        // A failed `ov_fopen` cleans its partial state; `ov_clear`
        // is valid only after success.
        if result != 0 {
            return Err(vorbis_error("ov_fopen", i64::from(result)));
        }
        let mut decoder = Self {
            storage: Some(storage),
            metadata: VorbisMetadata {
                sample_rate: 0,
                channels: 1,
                total_frames: 0,
                duration_seconds: 0.0,
                logical_streams: 0,
            },
        };
        if let Err(error) = decoder.read_metadata() {
            if let Some(mut storage) = decoder.storage.take() {
                // SAFETY: the storage opened successfully and is
                // cleared once on this failure path.
                unsafe {
                    ov_clear(&mut *storage);
                }
            }
            return Err(error);
        }
        Ok(decoder)
    }

    fn storage(&mut self) -> Result<*mut VorbisStorage, ClientError> {
        self.storage
            .as_mut()
            .map(|storage| &mut **storage as *mut VorbisStorage)
            .ok_or_else(|| ClientError::BadMedia("Vorbis decoder is closed".to_string()))
    }

    fn read_metadata(&mut self) -> Result<(), ClientError> {
        let storage = self.storage()?;
        let (sample_rate, channels) = stream_format(storage, 0)?;
        // SAFETY: the storage is open across each synchronous call.
        let logical_streams = safe_count(unsafe { ov_streams(storage) }, "ov_streams")?;
        if logical_streams == 0 || logical_streams > 65536 {
            return Err(ClientError::BadMedia(
                "Vorbis logical stream count is outside 1..65536".to_string(),
            ));
        }
        for link in 1..logical_streams {
            let next = stream_format(storage, link)?;
            if next != (sample_rate, channels) {
                return Err(ClientError::BadMedia(
                    "Vorbis chained streams change the PCM format".to_string(),
                ));
            }
        }
        let total_frames = safe_count(unsafe { ov_pcm_total(storage, -1) }, "ov_pcm_total")?;
        self.metadata = VorbisMetadata {
            sample_rate,
            channels,
            total_frames,
            duration_seconds: total_frames as f64 / f64::from(sample_rate),
            logical_streams,
        };
        Ok(())
    }

    /// Metadata.
    #[must_use]
    pub const fn metadata(&self) -> VorbisMetadata {
        self.metadata
    }

    /// Whether the decoder is closed.
    #[must_use]
    pub const fn is_closed(&self) -> bool {
        self.storage.is_none()
    }

    /// Current position in frames.
    pub fn position_frames(&self) -> Result<usize, ClientError> {
        let storage = self
            .storage
            .as_ref()
            .map(|storage| &**storage as *const VorbisStorage as *mut VorbisStorage)
            .ok_or_else(|| ClientError::BadMedia("Vorbis decoder is closed".to_string()))?;
        // SAFETY: the storage is open across the synchronous call;
        // `ov_pcm_tell` does not mutate decoder state.
        safe_count(unsafe { ov_pcm_tell(storage) }, "ov_pcm_tell")
    }

    /// Seek to a frame.
    pub fn seek(&mut self, frame: usize) -> Result<(), ClientError> {
        let storage = self.storage()?;
        if frame > self.metadata.total_frames {
            return Err(ClientError::BadMedia(
                "Vorbis seek frame is outside the stream".to_string(),
            ));
        }
        // SAFETY: the storage is open across the synchronous call.
        let result = unsafe { ov_pcm_seek(storage, frame as i64) };
        if result != 0 {
            return Err(vorbis_error("ov_pcm_seek", i64::from(result)));
        }
        Ok(())
    }

    /// Read up to `max_frames`, returning `None` only at EOF
    /// (`read`).
    pub fn read(&mut self, max_frames: usize) -> Result<Option<VorbisPcmChunk>, ClientError> {
        let storage = self.storage()?;
        if !(1..=1_048_576).contains(&max_frames) {
            return Err(ClientError::BadMedia(
                "Vorbis read frame limit must be in 1..1048576".to_string(),
            ));
        }
        let (channels, sample_rate) = (self.metadata.channels, self.metadata.sample_rate);
        let mut samples = vec![0i16; max_frames * usize::from(channels)];
        let mut bitstream = 0 as c_int;
        let mut sample_count = 0usize;
        while sample_count < samples.len() {
            let output = &mut samples[sample_count..];
            // SAFETY: the output borrows live samples across the
            // synchronous call; the storage is open.
            let count = unsafe {
                ov_read(
                    storage,
                    output.as_mut_ptr().cast::<c_char>(),
                    output.len() as c_int * 2,
                    c_int::from(cfg!(target_endian = "big")),
                    2,
                    1,
                    &mut bitstream,
                )
            };
            if count < 0 {
                return Err(vorbis_error("ov_read", count as i64));
            }
            if count == 0 {
                break;
            }
            if count as usize > output.len() * 2 || !(count as usize).is_multiple_of(usize::from(channels) * 2) {
                return Err(ClientError::BadMedia(
                    "Vorbis returned an invalid PCM byte count".to_string(),
                ));
            }
            if bitstream < 0 || bitstream as usize >= self.metadata.logical_streams {
                return Err(ClientError::BadMedia(
                    "Vorbis returned an invalid logical stream index".to_string(),
                ));
            }
            sample_count += count as usize / 2;
        }
        if sample_count == 0 {
            return Ok(None);
        }
        samples.truncate(sample_count);
        Ok(Some(VorbisPcmChunk {
            samples,
            frames: sample_count / usize::from(channels),
            sample_rate,
            channels,
        }))
    }

    /// Close the decoder.
    pub fn close(&mut self) -> Result<(), ClientError> {
        let Some(mut storage) = self.storage.take() else {
            return Ok(());
        };
        // SAFETY: the storage is open and cleared once.
        let result = unsafe { ov_clear(&mut *storage) };
        if result != 0 {
            return Err(vorbis_error("ov_clear", result as i64));
        }
        Ok(())
    }
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A Vorbis PCM stream (`VorbisPcmStream`).
pub struct VorbisPcmStream {
    decoder: VorbisDecoder,
    temp_dir: Option<PathBuf>,
}

impl Drop for VorbisPcmStream {
    fn drop(&mut self) {
        let _ = self.decoder.close();
        if let Some(dir) = self.temp_dir.take() {
            std::fs::remove_dir_all(dir).ok();
        }
    }
}

impl VorbisPcmStream {
    /// Open a stream over a file (`VorbisPcmStream.open`).
    pub fn open(path: &str) -> Result<Self, ClientError> {
        Ok(Self {
            decoder: VorbisDecoder::open(path)?,
            temp_dir: None,
        })
    }

    /// Open a stream over bytes (`VorbisPcmStream.fromBytes`): the
    /// bytes live in an owned, removed temporary file.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ClientError> {
        let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("quake-audio-{}-{id}", std::process::id()));
        std::fs::create_dir_all(&dir)
            .map_err(|error| ClientError::BadMedia(format!("Vorbis temporary file: {error}")))?;
        let path = dir.join("source.ogg");
        let result = std::fs::write(&path, bytes)
            .map_err(|error| ClientError::BadMedia(format!("Vorbis temporary file: {error}")))
            .and_then(|()| {
                VorbisDecoder::open(&path.to_string_lossy()).map(|decoder| Self {
                    decoder,
                    temp_dir: Some(dir.clone()),
                })
            });
        if result.is_err() {
            std::fs::remove_dir_all(&dir).ok();
        }
        result
    }

    /// Sample rate.
    #[must_use]
    pub fn sample_rate(&self) -> u32 {
        self.decoder.metadata.sample_rate
    }

    /// Channels.
    #[must_use]
    pub fn channels(&self) -> u8 {
        self.decoder.metadata.channels
    }

    /// Frame count.
    #[must_use]
    pub fn frame_count(&self) -> usize {
        self.decoder.metadata.total_frames
    }

    /// Current position in frames.
    pub fn position_frames(&self) -> Result<usize, ClientError> {
        self.decoder.position_frames()
    }

    /// Read up to `max_frames`.
    pub fn read(&mut self, max_frames: usize) -> Result<Option<VorbisPcmChunk>, ClientError> {
        self.decoder.read(max_frames)
    }

    /// Seek to a frame.
    pub fn seek(&mut self, frame: usize) -> Result<(), ClientError> {
        self.decoder.seek(frame)
    }

    /// Close the stream (removes the temporary file).
    pub fn close(&mut self) {
        let _ = self.decoder.close();
        if let Some(dir) = self.temp_dir.take() {
            std::fs::remove_dir_all(dir).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, size_of};
    use std::sync::Mutex;

    // Parallel tests share the temp dir; serialize temp-file tests.
    static TEMP_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn storage_layout_matches_abi() {
        assert_eq!(size_of::<VorbisStorage>(), 2048);
        assert_eq!(align_of::<VorbisStorage>(), 8);
    }

    #[test]
    fn path_and_state_errors() {
        let _guard = TEMP_LOCK.lock().unwrap();
        assert!(VorbisDecoder::open("").is_err());
        assert!(VorbisDecoder::open("bad\0path").is_err());
        assert!(VorbisDecoder::open("/nonexistent/source-1273.ogg").is_err());
        assert!(VorbisPcmStream::from_bytes(b"not vorbis").is_err());
        assert!(VorbisPcmStream::open("/nonexistent/source-1273.ogg").is_err());
    }

    #[test]
    fn temporary_files_are_removed() {
        let _guard = TEMP_LOCK.lock().unwrap();
        let before: Vec<_> = std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("quake-audio-"))
            .collect();
        assert!(VorbisPcmStream::from_bytes(b"not vorbis").is_err());
        let after: Vec<_> = std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("quake-audio-"))
            .collect();
        assert_eq!(before.len(), after.len());
    }
}
