//! Ogg Vorbis decoding through libvorbisfile.
//!
//! Port of donor `src/platform/vorbis.ts` (the `cd_ogg.ts` decode half,
//! separated from track/game ownership). Only the qualified 64-bit LP64 ABI
//! (Linux/macOS) is supported; the LP64 `OggVorbis_File` is 944 bytes, kept
//! in the source ports' 2048-byte reserve that only libvorbisfile touches.

use std::ffi::c_void;
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::ffi_util::{c_string, LoadedLibrary};
use crate::native_libraries::{NativeLibrary, NativeLibraryOptions};

/// Reserve backing `OggVorbis_File` storage (donor: 2048 bytes, 8-aligned).
const STORAGE_LEN: usize = 2048;
/// Maximum frames per [`VorbisDecoder::read`].
pub const MAX_READ_FRAMES: u32 = 1_048_576;
/// Default frames per [`VorbisDecoder::read`].
pub const DEFAULT_READ_FRAMES: u32 = 4096;

/// True on the qualified 64-bit LP64 ABI.
#[must_use]
pub fn vorbis_abi_supported() -> bool {
    #[cfg(all(
        any(target_os = "linux", target_os = "macos"),
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    {
        true
    }
    #[cfg(not(all(
        any(target_os = "linux", target_os = "macos"),
        any(target_arch = "x86_64", target_arch = "aarch64")
    )))]
    {
        false
    }
}

/// Typed Vorbis failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VorbisError {
    /// Operation name, e.g. `"ov_read"`.
    pub operation: String,
    /// libvorbisfile error code.
    pub code: i64,
}

impl VorbisError {
    fn new(operation: impl Into<String>, code: i64) -> Self {
        Self {
            operation: operation.into(),
            code,
        }
    }
}

impl std::fmt::Display for VorbisError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} failed with Vorbis error {}", self.operation, self.code)
    }
}

impl std::error::Error for VorbisError {}

impl From<VorbisError> for Error {
    fn from(error: VorbisError) -> Self {
        Self::Coded {
            operation: error.operation,
            code: error.code,
        }
    }
}

/// Stream metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct VorbisMetadata {
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Channel count (1 or 2).
    pub channels: u8,
    /// Total PCM frames across all chained streams.
    pub total_frames: u64,
    /// Duration in seconds.
    pub duration_seconds: f64,
    /// Logical stream count.
    pub logical_streams: u64,
}

/// One decoded PCM chunk. Samples belong to the caller and survive decoder
/// reads, seeks, and close.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VorbisPcmChunk {
    /// Interleaved native-endian samples.
    pub samples: Vec<i16>,
    /// Frame count.
    pub frames: u64,
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Channel count.
    pub channels: u8,
}

/// Narrow a libvorbisfile count, mapping negatives to [`VorbisError`].
pub fn safe_count(value: i64, name: &str) -> Result<u64> {
    if value < 0 {
        return Err(VorbisError::new(name, value).into());
    }
    if value > (1i64 << 53) - 1 {
        return Err(Error::OutOfRange(format!("{name} exceeds safe integer precision")));
    }
    Ok(value as u64)
}

/// Validate a read frame limit (1..=1048576).
pub fn validate_read_limit(max_frames: u32) -> Result<()> {
    if !(1..=MAX_READ_FRAMES).contains(&max_frames) {
        return Err(Error::OutOfRange(
            "Vorbis read frame limit must be in 1..1048576".to_string(),
        ));
    }
    Ok(())
}

/// Validate a seek target against the stream length.
pub fn validate_seek(frame: u64, total_frames: u64) -> Result<()> {
    if frame > total_frames {
        return Err(Error::OutOfRange("Vorbis seek frame is outside the stream".to_string()));
    }
    Ok(())
}

/// Validate a channel count from native metadata (1 or 2).
pub fn validate_channels(channels: i32) -> Result<u8> {
    if channels != 1 && channels != 2 {
        return Err(Error::InvalidInput("Vorbis output must be mono or stereo".to_string()));
    }
    Ok(channels as u8)
}

/// Validate a sample rate from native metadata (8000..=192000 Hz).
pub fn validate_sample_rate(sample_rate: u64) -> Result<u32> {
    if !(8000..=192000).contains(&sample_rate) {
        return Err(Error::InvalidInput(
            "Vorbis sample rate must be in 8000..192000 Hz".to_string(),
        ));
    }
    Ok(sample_rate as u32)
}

struct VorbisLib {
    _lib: LoadedLibrary,
    ov_fopen: unsafe extern "C" fn(*const u8, *mut u8) -> i32,
    ov_read: unsafe extern "C" fn(*mut u8, *mut u8, i32, i32, i32, i32, *mut i32) -> i64,
    ov_info: unsafe extern "C" fn(*mut u8, i32) -> *mut c_void,
    ov_streams: unsafe extern "C" fn(*mut u8) -> i64,
    ov_pcm_total: unsafe extern "C" fn(*mut u8, i32) -> i64,
    ov_pcm_tell: unsafe extern "C" fn(*mut u8) -> i64,
    ov_pcm_seek: unsafe extern "C" fn(*mut u8, i64) -> i32,
    ov_clear: unsafe extern "C" fn(*mut u8) -> i32,
}

impl VorbisLib {
    /// # Safety
    ///
    /// Resolved symbols are only invoked with the libvorbisfile ABI below.
    unsafe fn load(options: &NativeLibraryOptions) -> Result<Self> {
        if !vorbis_abi_supported() {
            return Err(Error::Unsupported(
                "Vorbis decoding requires the qualified 64-bit LP64 ABI".to_string(),
            ));
        }
        // SAFETY: loading maps the image without invoking its code.
        let lib = unsafe { LoadedLibrary::open(NativeLibrary::VorbisFile, options)? };
        macro_rules! sym {
            ($name:literal, $sig:ty) => {
                // SAFETY: the address is only read here.
                unsafe { lib.symbol::<$sig>(concat!($name, "\0").as_bytes())? }
            };
        }
        Ok(Self {
            ov_fopen: sym!("ov_fopen", unsafe extern "C" fn(*const u8, *mut u8) -> i32),
            ov_read: sym!(
                "ov_read",
                unsafe extern "C" fn(*mut u8, *mut u8, i32, i32, i32, i32, *mut i32) -> i64
            ),
            ov_info: sym!("ov_info", unsafe extern "C" fn(*mut u8, i32) -> *mut c_void),
            ov_streams: sym!("ov_streams", unsafe extern "C" fn(*mut u8) -> i64),
            ov_pcm_total: sym!("ov_pcm_total", unsafe extern "C" fn(*mut u8, i32) -> i64),
            ov_pcm_tell: sym!("ov_pcm_tell", unsafe extern "C" fn(*mut u8) -> i64),
            ov_pcm_seek: sym!("ov_pcm_seek", unsafe extern "C" fn(*mut u8, i64) -> i32),
            ov_clear: sym!("ov_clear", unsafe extern "C" fn(*mut u8) -> i32),
            _lib: lib,
        })
    }
}

// SAFETY: symbols are only called on the owning thread.
unsafe impl Send for VorbisLib {}
unsafe impl Sync for VorbisLib {}

/// An open Vorbis decoder.
pub struct VorbisDecoder {
    lib: Arc<VorbisLib>,
    /// 8-byte aligned `OggVorbis_File` reserve; only libvorbisfile accesses it.
    storage: Option<Box<[u64; STORAGE_LEN / 8]>>,
    metadata: VorbisMetadata,
}

impl VorbisDecoder {
    /// Open a decoder with live process discovery.
    pub fn open(path: &str) -> Result<Self> {
        Self::open_with(path, &NativeLibraryOptions::default())
    }

    /// Open with explicit library discovery (tests inject overrides).
    pub fn open_with(path: &str, options: &NativeLibraryOptions) -> Result<Self> {
        if path.is_empty() || path.contains('\0') {
            return Err(Error::InvalidInput(
                "Vorbis path must be nonempty and contain no NUL".to_string(),
            ));
        }
        // SAFETY: loading maps the image; calls below use validated arguments.
        let lib = Arc::new(unsafe { VorbisLib::load(options)? });
        let path_bytes = c_string(path)?;
        let mut storage = Box::new([0u64; STORAGE_LEN / 8]);
        // SAFETY: the path and storage are live for the call.
        let result = unsafe { (lib.ov_fopen)(path_bytes.as_ptr(), storage.as_mut_ptr().cast::<u8>()) };
        // Failed ov_fopen cleans its partial state; ov_clear is valid only after success.
        if result != 0 {
            return Err(VorbisError::new("ov_fopen", i64::from(result)).into());
        }
        let metadata = (|| {
            // SAFETY: the storage holds a live OggVorbis_File.
            unsafe {
                let (sample_rate, channels) = Self::stream_format(&lib, &mut storage, 0)?;
                let logical_streams = safe_count((lib.ov_streams)(storage.as_mut_ptr().cast::<u8>()), "ov_streams")?;
                if logical_streams == 0 || logical_streams > 65536 {
                    return Err(Error::InvalidInput(
                        "Vorbis logical stream count is outside 1..65536".to_string(),
                    ));
                }
                for index in 1..logical_streams {
                    let next = Self::stream_format(&lib, &mut storage, index as i32)?;
                    if next != (sample_rate, channels) {
                        return Err(Error::InvalidInput(
                            "Vorbis chained streams change the PCM format".to_string(),
                        ));
                    }
                }
                let total_frames = safe_count(
                    (lib.ov_pcm_total)(storage.as_mut_ptr().cast::<u8>(), -1),
                    "ov_pcm_total",
                )?;
                Ok(VorbisMetadata {
                    duration_seconds: total_frames as f64 / f64::from(sample_rate),
                    sample_rate,
                    channels,
                    total_frames,
                    logical_streams,
                })
            }
        })();
        match metadata {
            Ok(metadata) => Ok(Self {
                lib,
                storage: Some(storage),
                metadata,
            }),
            Err(error) => {
                // SAFETY: ov_clear is valid after successful ov_fopen.
                unsafe {
                    (lib.ov_clear)(storage.as_mut_ptr().cast::<u8>());
                }
                Err(error)
            }
        }
    }

    /// # Safety
    ///
    /// `storage` must hold a live `OggVorbis_File`.
    unsafe fn stream_format(
        lib: &VorbisLib,
        storage: &mut Box<[u64; STORAGE_LEN / 8]>,
        stream: i32,
    ) -> Result<(u32, u8)> {
        // SAFETY: caller guarantees a live OggVorbis_File.
        unsafe {
            let info = (lib.ov_info)(storage.as_mut_ptr().cast::<u8>(), stream);
            if info.is_null() {
                return Err(Error::InvalidInput("ov_info returned no stream metadata".to_string()));
            }
            let channels = std::ptr::read_unaligned(info.cast::<u8>().add(4).cast::<i32>());
            let rate = std::ptr::read_unaligned(info.cast::<u8>().add(8).cast::<i64>());
            Ok((
                validate_sample_rate(safe_count(rate, "Vorbis sample rate")?)?,
                validate_channels(channels)?,
            ))
        }
    }

    /// Stream metadata.
    #[must_use]
    pub fn metadata(&self) -> &VorbisMetadata {
        &self.metadata
    }

    /// Whether the decoder is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.storage.is_none()
    }

    fn opened(&self) -> Result<*mut u8> {
        self.storage.as_ref().map_or_else(
            || Err(Error::Closed("Vorbis decoder".to_string())),
            |storage| Ok(std::ptr::from_ref(storage.as_ref()).cast_mut().cast::<u8>()),
        )
    }

    /// Current position in frames.
    pub fn position_frames(&self) -> Result<u64> {
        let storage = self.opened()?;
        // SAFETY: the storage holds a live OggVorbis_File.
        unsafe { safe_count((self.lib.ov_pcm_tell)(storage), "ov_pcm_tell") }
    }

    /// Seek to a frame.
    pub fn seek(&mut self, frame: u64) -> Result<()> {
        let storage = self.opened()?;
        validate_seek(frame, self.metadata.total_frames)?;
        // SAFETY: the storage holds a live OggVorbis_File.
        let result = unsafe { (self.lib.ov_pcm_seek)(storage, frame as i64) };
        if result != 0 {
            return Err(VorbisError::new("ov_pcm_seek", i64::from(result)).into());
        }
        Ok(())
    }

    /// Read up to `max_frames`, returning `None` only at EOF. Corrupt packets fail explicitly.
    pub fn read(&mut self, max_frames: u32) -> Result<Option<VorbisPcmChunk>> {
        let storage = self.opened()?;
        validate_read_limit(max_frames)?;
        let channels = self.metadata.channels;
        let sample_rate = self.metadata.sample_rate;
        let mut samples = vec![0i16; max_frames as usize * usize::from(channels)];
        let mut bitstream = 0i32;
        let mut sample_count = 0usize;
        while sample_count < samples.len() {
            let output = &mut samples[sample_count..];
            // SAFETY: the storage holds a live OggVorbis_File; the output
            // buffer and bitstream out-pointer are live for the call.
            let count = unsafe {
                (self.lib.ov_read)(
                    storage,
                    output.as_mut_ptr().cast::<u8>(),
                    (output.len() * 2) as i32,
                    i32::from(cfg!(target_endian = "big")),
                    2,
                    1,
                    &mut bitstream,
                )
            };
            if count < 0 {
                return Err(VorbisError::new("ov_read", count).into());
            }
            if count == 0 {
                break;
            }
            if count as usize > output.len() * 2 || !(count as usize).is_multiple_of(usize::from(channels) * 2) {
                return Err(Error::InvalidInput(
                    "Vorbis returned an invalid PCM byte count".to_string(),
                ));
            }
            if bitstream < 0 || bitstream as u64 >= self.metadata.logical_streams {
                return Err(Error::InvalidInput(
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
            frames: sample_count as u64 / u64::from(channels),
            sample_rate,
            channels,
        }))
    }

    /// Close the decoder. Idempotent.
    pub fn close(&mut self) -> Result<()> {
        let Some(mut storage) = self.storage.take() else {
            return Ok(());
        };
        // SAFETY: ov_clear is valid after successful ov_fopen.
        let result = unsafe { (self.lib.ov_clear)(storage.as_mut_ptr().cast::<u8>()) };
        if result != 0 {
            return Err(VorbisError::new("ov_clear", i64::from(result)).into());
        }
        Ok(())
    }
}

impl Drop for VorbisDecoder {
    fn drop(&mut self) {
        self.close().ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn missing_lib() -> NativeLibraryOptions {
        let mut environment = std::collections::HashMap::new();
        environment.insert(
            "QUAKE_VORBISFILE_LIBRARY".to_string(),
            "/nonexistent-qa-platform/libvorbisfile.so".to_string(),
        );
        NativeLibraryOptions {
            environment: Some(environment),
            ..NativeLibraryOptions::default()
        }
    }

    #[test]
    fn counts_and_limits_validate() {
        assert_eq!(safe_count(44100, "ov_pcm_total").unwrap(), 44100);
        assert!(safe_count(-132, "ov_read").is_err());
        assert!(safe_count(1i64 << 60, "ov_pcm_total").is_err());
        assert!(validate_read_limit(4096).is_ok());
        assert!(validate_read_limit(0).is_err());
        assert!(validate_read_limit(MAX_READ_FRAMES + 1).is_err());
        assert!(validate_seek(10, 100).is_ok());
        assert!(validate_seek(101, 100).is_err());
        assert_eq!(validate_channels(2).unwrap(), 2);
        assert!(validate_channels(6).is_err());
        assert_eq!(validate_sample_rate(44100).unwrap(), 44100);
        assert!(validate_sample_rate(4000).is_err());
        assert!(vorbis_abi_supported());
    }

    #[test]
    fn missing_library_names_vorbisfile() {
        assert!(VorbisDecoder::open_with("", &missing_lib()).is_err());
        let Err(error) = VorbisDecoder::open_with("/audio/track01.ogg", &missing_lib()) else {
            panic!("expected failure")
        };
        assert!(error.to_string().contains("vorbisfile"), "{error}");
    }

    #[test]
    fn error_displays_operation_and_code() {
        let error = VorbisError::new("ov_fopen", -128);
        assert_eq!(error.to_string(), "ov_fopen failed with Vorbis error -128");
    }

    #[test]
    fn live_open_reports_honestly() {
        // A missing file exercises the real FFI failure path when the library
        // is present (ov_fopen fails), or the absence signal without it.
        match VorbisDecoder::open("/nonexistent-qa-platform/track01.ogg") {
            Ok(_) => panic!("missing file decoded"),
            Err(error) => assert!(!error.to_string().is_empty(), "{error}"),
        }
    }
}
