//! SDL audio output (SDL3 preferred, SDL2 fallback).
//!
//! Port of donor `src/platform/audio.ts`: explicit SDL3 overrides stay
//! authoritative, while a missing SDL3 library falls back to SDL2. An
//! unavailable native device leaves source `SNDDMA_Init` unstarted, reported
//! here as [`Error::Unavailable`]. Both adapters expose exact input-byte
//! counts while paused, including resampling tails.

use std::ffi::c_void;
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::ffi_util::{c_string, c_string_lossy, sdl_error, LoadedLibrary};
use crate::native_libraries::{NativeLibrary, NativeLibraryOptions};

const AUDIO_SUBSYSTEM: u32 = 0x10;

/// Signed 16-bit native-endian sample format.
#[cfg(target_endian = "little")]
pub const SIGNED16_NATIVE: i32 = 0x8010;
/// Signed 16-bit native-endian sample format.
#[cfg(target_endian = "big")]
pub const SIGNED16_NATIVE: i32 = 0x9010;
/// Unsigned 8-bit sample format.
pub const UNSIGNED8: i32 = 0x0008;
/// SDL3 default output device id.
const SDL3_DEFAULT_OUTPUT: u32 = 0xffff_ffff;

/// Channel count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioChannels {
    /// Mono.
    Mono = 1,
    /// Stereo.
    Stereo = 2,
}

impl AudioChannels {
    fn count(self) -> u32 {
        self as u32
    }
}

/// Sample width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioSampleBits {
    /// Unsigned 8-bit.
    B8 = 8,
    /// Signed 16-bit native-endian.
    B16 = 16,
}

impl AudioSampleBits {
    fn bytes(self) -> u32 {
        self as u32 / 8
    }

    fn format(self) -> i32 {
        match self {
            Self::B16 => SIGNED16_NATIVE,
            Self::B8 => UNSIGNED8,
        }
    }
}

/// Audio device open options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SdlAudioOptions {
    /// Sample rate in Hz (8000..=192000).
    pub sample_rate: u32,
    /// Channel count.
    pub channels: AudioChannels,
    /// Sample width (default 16-bit).
    pub sample_bits: AudioSampleBits,
    /// Output device name, or `None` for the default.
    pub device_name: Option<String>,
    /// Advisory buffer duration in input frames (1..=32768, default 1024).
    pub buffer_frames: u32,
}

impl Default for SdlAudioOptions {
    fn default() -> Self {
        Self {
            sample_rate: 44100,
            channels: AudioChannels::Stereo,
            sample_bits: AudioSampleBits::B16,
            device_name: None,
            buffer_frames: 1024,
        }
    }
}

impl SdlAudioOptions {
    fn validate(&self) -> Result<()> {
        if self.sample_rate < 8000 || self.sample_rate > 192000 {
            return Err(Error::OutOfRange(
                "audio sample rate must be an integer in 8000..192000 Hz".to_string(),
            ));
        }
        if let Some(name) = &self.device_name {
            if name.is_empty() || name.contains('\0') {
                return Err(Error::InvalidInput(
                    "audio device name must be nonempty and contain no NUL".to_string(),
                ));
            }
        }
        if self.buffer_frames < 1 || self.buffer_frames > 32768 {
            return Err(Error::OutOfRange(
                "audio buffer frames must be an integer in 1..32768".to_string(),
            ));
        }
        Ok(())
    }
}

/// SDL2 buffer rounding: next power of two clamped to 64..=32768.
pub fn sdl2_requested_frames(buffer_frames: u32) -> u32 {
    buffer_frames.next_power_of_two().clamp(64, 32768)
}

/// Resample a hardware frame count into input frames, rounding up.
pub fn resampled_frames(hardware_frames: u32, sample_rate: u32, hardware_rate: u32) -> u64 {
    (u64::from(hardware_frames) * u64::from(sample_rate)).div_ceil(u64::from(hardware_rate))
}

struct Sdl2Audio {
    _lib: LoadedLibrary,
    sdl_set_main_ready: unsafe extern "C" fn(),
    sdl_init_sub_system: unsafe extern "C" fn(u32) -> i32,
    sdl_set_hint: unsafe extern "C" fn(*const u8, *const u8) -> i32,
    sdl_get_hint: unsafe extern "C" fn(*const u8) -> *mut c_void,
    sdl_quit_sub_system: unsafe extern "C" fn(u32),
    sdl_get_error: unsafe extern "C" fn() -> *const u8,
    sdl_get_num_audio_devices: unsafe extern "C" fn(i32) -> i32,
    sdl_get_audio_device_name: unsafe extern "C" fn(i32, i32) -> *const u8,
    sdl_get_performance_counter: unsafe extern "C" fn() -> u64,
    sdl_get_performance_frequency: unsafe extern "C" fn() -> u64,
    sdl_open_audio_device: unsafe extern "C" fn(*const c_void, i32, *const u8, *mut u8, i32) -> u32,
    sdl_queue_audio: unsafe extern "C" fn(u32, *const u8, u32) -> i32,
    sdl_get_queued_audio_size: unsafe extern "C" fn(u32) -> u32,
    sdl_pause_audio_device: unsafe extern "C" fn(u32, i32),
    sdl_clear_queued_audio: unsafe extern "C" fn(u32),
    sdl_close_audio_device: unsafe extern "C" fn(u32),
}

struct Sdl3Audio {
    _lib: LoadedLibrary,
    sdl_set_main_ready: unsafe extern "C" fn(),
    sdl_init_sub_system: unsafe extern "C" fn(u32) -> bool,
    sdl_set_hint: unsafe extern "C" fn(*const u8, *const u8) -> bool,
    sdl_get_hint: unsafe extern "C" fn(*const u8) -> *mut c_void,
    sdl_quit_sub_system: unsafe extern "C" fn(u32),
    sdl_get_error: unsafe extern "C" fn() -> *const u8,
    sdl_get_audio_playback_devices: unsafe extern "C" fn(*mut i32) -> *mut u32,
    sdl_get_audio_device_name: unsafe extern "C" fn(u32) -> *const u8,
    sdl_free: unsafe extern "C" fn(*mut c_void),
    sdl_get_audio_device_format: unsafe extern "C" fn(u32, *mut u8, *mut i32) -> bool,
    sdl_get_audio_stream_device: unsafe extern "C" fn(*mut c_void) -> u32,
    sdl_get_audio_stream_format: unsafe extern "C" fn(*mut c_void, *mut u8, *mut c_void) -> bool,
    sdl_open_audio_device_stream: unsafe extern "C" fn(u32, *const u8, *const c_void, *const c_void) -> *mut c_void,
    sdl_get_audio_stream_queued: unsafe extern "C" fn(*mut c_void) -> i32,
    sdl_put_audio_stream_data: unsafe extern "C" fn(*mut c_void, *const u8, i32) -> bool,
    sdl_clear_audio_stream: unsafe extern "C" fn(*mut c_void) -> bool,
    sdl_pause_audio_stream_device: unsafe extern "C" fn(*mut c_void) -> bool,
    sdl_resume_audio_stream_device: unsafe extern "C" fn(*mut c_void) -> bool,
    sdl_destroy_audio_stream: unsafe extern "C" fn(*mut c_void),
    sdl_get_performance_counter: unsafe extern "C" fn() -> u64,
    sdl_get_performance_frequency: unsafe extern "C" fn() -> u64,
}

macro_rules! define_audio_loader {
    ($struct:ident, $kind:expr, $($field:ident : $cname:literal;)*) => {
        impl $struct {
            /// # Safety
            ///
            /// Resolved symbols are only invoked with the SDL ABI below.
            unsafe fn load(options: &NativeLibraryOptions) -> Result<Arc<Self>> {
                // SAFETY: loading maps the image without invoking its code.
                let lib = unsafe { LoadedLibrary::open($kind, options)? };
                let this = Arc::new(Self {
                    $( $field: unsafe { lib.symbol(concat!($cname, "\0").as_bytes())? }, )*
                    _lib: lib,
                });
                // SAFETY: no arguments; required before any other SDL call.
                unsafe { (this.sdl_set_main_ready)(); }
                Ok(this)
            }

            /// # Safety
            ///
            /// SDL calls below uphold their own argument contracts.
            unsafe fn error(&self) -> String {
                // SAFETY: SDL guarantees a valid thread-local error string.
                unsafe { sdl_error(self.sdl_get_error) }
            }
        }
    };
}

define_audio_loader! { Sdl2Audio, NativeLibrary::Sdl2,
    sdl_set_main_ready: "SDL_SetMainReady";
    sdl_init_sub_system: "SDL_InitSubSystem";
    sdl_set_hint: "SDL_SetHint";
    sdl_get_hint: "SDL_GetHint";
    sdl_quit_sub_system: "SDL_QuitSubSystem";
    sdl_get_error: "SDL_GetError";
    sdl_get_num_audio_devices: "SDL_GetNumAudioDevices";
    sdl_get_audio_device_name: "SDL_GetAudioDeviceName";
    sdl_get_performance_counter: "SDL_GetPerformanceCounter";
    sdl_get_performance_frequency: "SDL_GetPerformanceFrequency";
    sdl_open_audio_device: "SDL_OpenAudioDevice";
    sdl_queue_audio: "SDL_QueueAudio";
    sdl_get_queued_audio_size: "SDL_GetQueuedAudioSize";
    sdl_pause_audio_device: "SDL_PauseAudioDevice";
    sdl_clear_queued_audio: "SDL_ClearQueuedAudio";
    sdl_close_audio_device: "SDL_CloseAudioDevice";
}

define_audio_loader! { Sdl3Audio, NativeLibrary::Sdl3,
    sdl_set_main_ready: "SDL_SetMainReady";
    sdl_init_sub_system: "SDL_InitSubSystem";
    sdl_set_hint: "SDL_SetHint";
    sdl_get_hint: "SDL_GetHint";
    sdl_quit_sub_system: "SDL_QuitSubSystem";
    sdl_get_error: "SDL_GetError";
    sdl_get_audio_playback_devices: "SDL_GetAudioPlaybackDevices";
    sdl_get_audio_device_name: "SDL_GetAudioDeviceName";
    sdl_free: "SDL_free";
    sdl_get_audio_device_format: "SDL_GetAudioDeviceFormat";
    sdl_get_audio_stream_device: "SDL_GetAudioStreamDevice";
    sdl_get_audio_stream_format: "SDL_GetAudioStreamFormat";
    sdl_open_audio_device_stream: "SDL_OpenAudioDeviceStream";
    sdl_get_audio_stream_queued: "SDL_GetAudioStreamQueued";
    sdl_put_audio_stream_data: "SDL_PutAudioStreamData";
    sdl_clear_audio_stream: "SDL_ClearAudioStream";
    sdl_pause_audio_stream_device: "SDL_PauseAudioStreamDevice";
    sdl_resume_audio_stream_device: "SDL_ResumeAudioStreamDevice";
    sdl_destroy_audio_stream: "SDL_DestroyAudioStream";
    sdl_get_performance_counter: "SDL_GetPerformanceCounter";
    sdl_get_performance_frequency: "SDL_GetPerformanceFrequency";
}

/// Set a required hint, accepting a pre-existing equal value.
unsafe fn required_hint(
    set: impl Fn(*const u8, *const u8) -> bool,
    get: impl Fn(*const u8) -> *mut c_void,
    name: &str,
    value: &str,
    error: impl Fn() -> String,
) -> Result<()> {
    let key = c_string(name)?;
    let replacement = c_string(value)?;
    // SAFETY: hint buffers are live for the calls.
    unsafe {
        if set(key.as_ptr(), replacement.as_ptr()) {
            return Ok(());
        }
        let current = get(key.as_ptr());
        if !current.is_null() && c_string_lossy(current.cast::<u8>()) == value {
            return Ok(());
        }
    }
    Err(Error::native(format!("SDL_SetHint {name}"), error()))
}

trait AudioPort: Send {
    fn buffer_frames(&self) -> u64;
    fn queued_bytes(&self) -> Result<u64>;
    fn put(&self, samples: &[u8]) -> Result<()>;
    fn clear(&self) -> Result<()>;
    fn pause(&self) -> Result<()>;
    fn resume(&self) -> Result<()>;
    fn close_box(self: Box<Self>);
}

struct Sdl2Port {
    sdl: Arc<Sdl2Audio>,
    device: u32,
    buffer_frames: u64,
}

impl AudioPort for Sdl2Port {
    fn buffer_frames(&self) -> u64 {
        self.buffer_frames
    }

    fn queued_bytes(&self) -> Result<u64> {
        // SAFETY: the device id is live.
        Ok(unsafe { u64::from((self.sdl.sdl_get_queued_audio_size)(self.device)) })
    }

    fn put(&self, samples: &[u8]) -> Result<()> {
        // SAFETY: the device id and sample buffer are live.
        unsafe {
            let result = (self.sdl.sdl_queue_audio)(self.device, samples.as_ptr(), samples.len() as u32);
            if result < 0 {
                return Err(Error::native("SDL_QueueAudio", self.sdl.error()));
            }
        }
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        // SAFETY: the device id is live.
        unsafe {
            (self.sdl.sdl_clear_queued_audio)(self.device);
        }
        Ok(())
    }

    fn pause(&self) -> Result<()> {
        // SAFETY: the device id is live.
        unsafe {
            (self.sdl.sdl_pause_audio_device)(self.device, 1);
        }
        Ok(())
    }

    fn resume(&self) -> Result<()> {
        // SAFETY: the device id is live.
        unsafe {
            (self.sdl.sdl_pause_audio_device)(self.device, 0);
        }
        Ok(())
    }

    fn close_box(self: Box<Self>) {
        // SAFETY: the device id is live until this call.
        unsafe {
            (self.sdl.sdl_close_audio_device)(self.device);
        }
    }
}

struct Sdl3Port {
    sdl: Arc<Sdl3Audio>,
    stream: *mut c_void,
    buffer_frames: u64,
}

// SAFETY: the stream is only used on the owning thread.
unsafe impl Send for Sdl3Port {}

impl AudioPort for Sdl3Port {
    fn buffer_frames(&self) -> u64 {
        self.buffer_frames
    }

    fn queued_bytes(&self) -> Result<u64> {
        // SAFETY: the stream is live.
        let count = unsafe { (self.sdl.sdl_get_audio_stream_queued)(self.stream) };
        if count < 0 {
            // SAFETY: error read immediately after the failing call.
            return Err(Error::native("SDL_GetAudioStreamQueued", unsafe { self.sdl.error() }));
        }
        Ok(count as u64)
    }

    fn put(&self, samples: &[u8]) -> Result<()> {
        // SAFETY: the stream and sample buffer are live.
        let ok = unsafe { (self.sdl.sdl_put_audio_stream_data)(self.stream, samples.as_ptr(), samples.len() as i32) };
        if !ok {
            // SAFETY: error read immediately after the failing call.
            return Err(Error::native("SDL_PutAudioStreamData", unsafe { self.sdl.error() }));
        }
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        // SAFETY: the stream is live.
        let ok = unsafe { (self.sdl.sdl_clear_audio_stream)(self.stream) };
        if !ok {
            // SAFETY: error read immediately after the failing call.
            return Err(Error::native("SDL_ClearAudioStream", unsafe { self.sdl.error() }));
        }
        Ok(())
    }

    fn pause(&self) -> Result<()> {
        // SAFETY: the stream is live.
        let ok = unsafe { (self.sdl.sdl_pause_audio_stream_device)(self.stream) };
        if !ok {
            // SAFETY: error read immediately after the failing call.
            return Err(Error::native("SDL_PauseAudioStreamDevice", unsafe { self.sdl.error() }));
        }
        Ok(())
    }

    fn resume(&self) -> Result<()> {
        // SAFETY: the stream is live.
        let ok = unsafe { (self.sdl.sdl_resume_audio_stream_device)(self.stream) };
        if !ok {
            // SAFETY: error read immediately after the failing call.
            return Err(Error::native("SDL_ResumeAudioStreamDevice", unsafe {
                self.sdl.error()
            }));
        }
        Ok(())
    }

    fn close_box(self: Box<Self>) {
        // SAFETY: the stream is live until this call.
        unsafe {
            (self.sdl.sdl_destroy_audio_stream)(self.stream);
        }
    }
}

enum Backend {
    Sdl2(Arc<Sdl2Audio>),
    Sdl3(Arc<Sdl3Audio>),
}

impl Backend {
    /// Select SDL3, falling back to SDL2 unless an explicit SDL3 override is set.
    fn select(lib_options: &NativeLibraryOptions) -> Result<Self> {
        // SAFETY: loading maps images without invoking their code.
        match unsafe { Sdl3Audio::load(lib_options) } {
            Ok(audio) => Ok(Self::Sdl3(audio)),
            Err(error) => {
                // Explicit SDL3 overrides remain authoritative. Device failures
                // never select another backend.
                if lib_options.is_override_set(NativeLibrary::Sdl3) {
                    return Err(error);
                }
                // SAFETY: loading maps the image without invoking its code.
                unsafe { Sdl2Audio::load(lib_options) }.map(Self::Sdl2)
            }
        }
    }

    fn initialize(&self) -> Result<()> {
        match self {
            // SAFETY: hint buffers and subsystem ids are validated.
            Self::Sdl2(sdl) => unsafe {
                required_hint(
                    |name, value| (sdl.sdl_set_hint)(name, value) == 1,
                    |name| (sdl.sdl_get_hint)(name),
                    "SDL_NO_SIGNAL_HANDLERS",
                    "1",
                    || sdl.error(),
                )?;
                if (sdl.sdl_init_sub_system)(AUDIO_SUBSYSTEM) < 0 {
                    return Err(Error::unavailable(
                        "sdl2",
                        format!("SDL_InitSubSystem AUDIO: {}", sdl.error()),
                    ));
                }
                Ok(())
            },
            // SAFETY: hint buffers and subsystem ids are validated.
            Self::Sdl3(sdl) => unsafe {
                required_hint(
                    |name, value| (sdl.sdl_set_hint)(name, value),
                    |name| (sdl.sdl_get_hint)(name),
                    "SDL_NO_SIGNAL_HANDLERS",
                    "1",
                    || sdl.error(),
                )?;
                if std::env::var("SDL_AUDIO_DRIVER").is_err() {
                    if let Ok(driver) = std::env::var("SDL_AUDIODRIVER") {
                        required_hint(
                            |name, value| (sdl.sdl_set_hint)(name, value),
                            |name| (sdl.sdl_get_hint)(name),
                            "SDL_AUDIO_DRIVER",
                            &driver,
                            || sdl.error(),
                        )?;
                    }
                }
                if !(sdl.sdl_init_sub_system)(AUDIO_SUBSYSTEM) {
                    return Err(Error::unavailable(
                        "sdl3",
                        format!("SDL_InitSubSystem AUDIO: {}", sdl.error()),
                    ));
                }
                Ok(())
            },
        }
    }

    fn quit(&self) {
        match self {
            // SAFETY: quits the subsystem this backend initialized.
            Self::Sdl2(sdl) => unsafe {
                (sdl.sdl_quit_sub_system)(AUDIO_SUBSYSTEM);
            },
            // SAFETY: quits the subsystem this backend initialized.
            Self::Sdl3(sdl) => unsafe {
                (sdl.sdl_quit_sub_system)(AUDIO_SUBSYSTEM);
            },
        }
    }

    fn names(&self) -> Result<Vec<String>> {
        match self {
            // SAFETY: device indexes are validated against the count.
            Self::Sdl2(sdl) => unsafe {
                let count = (sdl.sdl_get_num_audio_devices)(0);
                let mut names = Vec::new();
                for index in 0..count.max(0) {
                    let name = c_string_lossy((sdl.sdl_get_audio_device_name)(index, 0));
                    if name.is_empty() {
                        return Err(Error::native("SDL_GetAudioDeviceName", sdl.error()));
                    }
                    names.push(name);
                }
                Ok(names)
            },
            Self::Sdl3(sdl) => Ok(unsafe { Self::sdl3_devices(sdl)? }
                .into_iter()
                .map(|(_, name)| name)
                .collect()),
        }
    }

    /// # Safety
    ///
    /// The SDL3 handle must be loaded and initialized.
    unsafe fn sdl3_devices(sdl: &Sdl3Audio) -> Result<Vec<(u32, String)>> {
        // SAFETY: the allocation protocol below matches SDL3 exactly.
        unsafe {
            let mut count = 0i32;
            let allocation = (sdl.sdl_get_audio_playback_devices)(&mut count);
            if allocation.is_null() {
                return Err(Error::unavailable(
                    "sdl3",
                    format!("SDL_GetAudioPlaybackDevices: {}", sdl.error()),
                ));
            }
            let result = (|| {
                if !(0..=65536).contains(&count) {
                    return Err(Error::InvalidInput("invalid SDL playback device count".to_string()));
                }
                let ids = std::slice::from_raw_parts(allocation, count as usize);
                let mut devices = Vec::with_capacity(count as usize);
                for &id in ids {
                    let name = c_string_lossy((sdl.sdl_get_audio_device_name)(id));
                    if id == 0 || name.is_empty() {
                        return Err(Error::native("SDL_GetAudioDeviceName", sdl.error()));
                    }
                    devices.push((id, name));
                }
                Ok(devices)
            })();
            (sdl.sdl_free)(allocation.cast::<c_void>());
            result
        }
    }

    fn open(&self, options: &SdlAudioOptions) -> Result<Box<dyn AudioPort>> {
        match self {
            Self::Sdl2(sdl) => Self::open_sdl2(sdl, options),
            Self::Sdl3(sdl) => Self::open_sdl3(sdl, options),
        }
    }

    fn open_sdl2(sdl: &Arc<Sdl2Audio>, options: &SdlAudioOptions) -> Result<Box<dyn AudioPort>> {
        // SAFETY: spec buffers are live for each call; results are checked.
        unsafe {
            // SDL2's 64-bit AudioSpec differs from SDL3: freq0,format4,channels6,samples8.
            let mut desired = [0u8; 32];
            let mut obtained = [0u8; 32];
            let format = options.sample_bits.format();
            desired[0..4].copy_from_slice(&options.sample_rate.to_ne_bytes());
            desired[4..6].copy_from_slice(&(format as u16).to_ne_bytes());
            desired[6] = options.channels.count() as u8;
            let requested = sdl2_requested_frames(options.buffer_frames);
            desired[8..10].copy_from_slice(&(requested as u16).to_ne_bytes());
            if options.device_name.is_some() {
                (sdl.sdl_get_num_audio_devices)(0);
            }
            let name = options.device_name.as_deref().map(c_string).transpose()?;
            let name_ptr = name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr().cast::<c_void>());
            let device = (sdl.sdl_open_audio_device)(name_ptr, 0, desired.as_ptr(), obtained.as_mut_ptr(), 0);
            if device == 0 {
                return Err(Error::unavailable(
                    "sdl2",
                    format!("SDL_OpenAudioDevice: {}", sdl.error()),
                ));
            }
            let result = (|| {
                let freq = i32::from_ne_bytes(obtained[0..4].try_into().expect("spec"));
                let actual_format = u16::from_ne_bytes(obtained[4..6].try_into().expect("spec"));
                if freq as u32 != options.sample_rate
                    || actual_format != format as u16
                    || obtained[6] != options.channels.count() as u8
                {
                    return Err(Error::InvalidInput(
                        "SDL changed the requested audio format".to_string(),
                    ));
                }
                let buffer_frames = u16::from_ne_bytes(obtained[8..10].try_into().expect("spec"));
                let size = u32::from_ne_bytes(obtained[12..16].try_into().expect("spec"));
                let frame_bytes = u32::from(buffer_frames) * options.channels.count() * options.sample_bits.bytes();
                let silence = if options.sample_bits == AudioSampleBits::B16 {
                    0
                } else {
                    128
                };
                if buffer_frames == 0 || size != frame_bytes || obtained[7] != silence {
                    return Err(Error::InvalidInput(
                        "SDL returned an invalid audio buffer specification".to_string(),
                    ));
                }
                Ok(buffer_frames)
            })();
            match result {
                Ok(buffer_frames) => Ok(Box::new(Sdl2Port {
                    sdl: Arc::clone(sdl),
                    device,
                    buffer_frames: u64::from(buffer_frames),
                })),
                Err(error) => {
                    (sdl.sdl_close_audio_device)(device);
                    Err(error)
                }
            }
        }
    }

    fn open_sdl3(sdl: &Arc<Sdl3Audio>, options: &SdlAudioOptions) -> Result<Box<dyn AudioPort>> {
        // SAFETY: spec buffers are live for each call; results are checked.
        unsafe {
            let id = match &options.device_name {
                None => SDL3_DEFAULT_OUTPUT,
                Some(wanted) => {
                    let found = Self::sdl3_devices(sdl)?
                        .into_iter()
                        .find(|(_, name)| name == wanted)
                        .map(|(id, _)| id);
                    match found {
                        Some(id) => id,
                        None => {
                            return Err(Error::unavailable(
                                "sdl3",
                                format!("audio output is unavailable: {wanted}"),
                            ));
                        }
                    }
                }
            };
            let mut spec = [0u8; 12];
            let mut frames = 0i32;
            if !(sdl.sdl_get_audio_device_format)(id, spec.as_mut_ptr(), &mut frames) {
                return Err(Error::native("SDL_GetAudioDeviceFormat preferred", sdl.error()));
            }
            let preferred_rate = i32::from_ne_bytes(spec[8..12].try_into().expect("spec"));
            if preferred_rate <= 0 {
                return Err(Error::InvalidInput(
                    "SDL returned an invalid preferred audio rate".to_string(),
                ));
            }
            // The hint is in hardware frames; the public request is in input frames.
            let hint_name = c_string("SDL_AUDIO_DEVICE_SAMPLE_FRAMES")?;
            let hint_value = c_string(
                &resampled_frames(options.buffer_frames, preferred_rate as u32, options.sample_rate).to_string(),
            )?;
            (sdl.sdl_set_hint)(hint_name.as_ptr(), hint_value.as_ptr());
            let format = options.sample_bits.format();
            spec[0..4].copy_from_slice(&format.to_ne_bytes());
            spec[4..8].copy_from_slice(&(options.channels.count() as i32).to_ne_bytes());
            spec[8..12].copy_from_slice(&(options.sample_rate as i32).to_ne_bytes());
            let stream = (sdl.sdl_open_audio_device_stream)(id, spec.as_ptr(), std::ptr::null(), std::ptr::null());
            if stream.is_null() {
                return Err(Error::unavailable(
                    "sdl3",
                    format!("SDL_OpenAudioDeviceStream: {}", sdl.error()),
                ));
            }
            let result = (|| {
                if !(sdl.sdl_get_audio_stream_format)(stream, spec.as_mut_ptr(), std::ptr::null_mut()) {
                    return Err(Error::native("SDL_GetAudioStreamFormat", sdl.error()));
                }
                let actual_format = i32::from_ne_bytes(spec[0..4].try_into().expect("spec"));
                let actual_channels = i32::from_ne_bytes(spec[4..8].try_into().expect("spec"));
                let actual_rate = i32::from_ne_bytes(spec[8..12].try_into().expect("spec"));
                if actual_format != format
                    || actual_channels as u32 != options.channels.count()
                    || actual_rate as u32 != options.sample_rate
                {
                    return Err(Error::InvalidInput(
                        "SDL changed the requested audio input format".to_string(),
                    ));
                }
                let device = (sdl.sdl_get_audio_stream_device)(stream);
                if !(sdl.sdl_get_audio_device_format)(device, spec.as_mut_ptr(), &mut frames) {
                    return Err(Error::native("SDL_GetAudioDeviceFormat opened", sdl.error()));
                }
                let hardware_rate = i32::from_ne_bytes(spec[8..12].try_into().expect("spec"));
                if hardware_rate <= 0 || frames <= 0 {
                    return Err(Error::InvalidInput(
                        "SDL returned an invalid audio buffer specification".to_string(),
                    ));
                }
                Ok(resampled_frames(
                    frames as u32,
                    options.sample_rate,
                    hardware_rate as u32,
                ))
            })();
            match result {
                Ok(buffer_frames) => Ok(Box::new(Sdl3Port {
                    sdl: Arc::clone(sdl),
                    stream,
                    buffer_frames,
                })),
                Err(error) => {
                    (sdl.sdl_destroy_audio_stream)(stream);
                    Err(error)
                }
            }
        }
    }

    fn counter(&self) -> u64 {
        match self {
            // SAFETY: no arguments.
            Self::Sdl2(sdl) => unsafe { (sdl.sdl_get_performance_counter)() },
            // SAFETY: no arguments.
            Self::Sdl3(sdl) => unsafe { (sdl.sdl_get_performance_counter)() },
        }
    }

    fn frequency(&self) -> u64 {
        match self {
            // SAFETY: no arguments.
            Self::Sdl2(sdl) => unsafe { (sdl.sdl_get_performance_frequency)() },
            // SAFETY: no arguments.
            Self::Sdl3(sdl) => unsafe { (sdl.sdl_get_performance_frequency)() },
        }
    }
}

/// Audio device playback state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioState {
    /// Open but not playing.
    Paused,
    /// Playing.
    Playing,
    /// Closed.
    Closed,
}

/// An open SDL audio output device.
pub struct SdlAudioDevice {
    backend: Backend,
    port: Option<Box<dyn AudioPort>>,
    clock_frequency: u64,
    elapsed_ticks: u64,
    playing_since: Option<u64>,
    max_queued_frames: u64,
    sample_rate: u32,
    channels: AudioChannels,
    sample_bits: AudioSampleBits,
    device_name: Option<String>,
    buffer_frames: u64,
}

impl SdlAudioDevice {
    /// List output device names with live process discovery.
    pub fn output_device_names() -> Result<Vec<String>> {
        Self::output_device_names_with(&NativeLibraryOptions::default())
    }

    /// List output device names with explicit library discovery.
    pub fn output_device_names_with(lib_options: &NativeLibraryOptions) -> Result<Vec<String>> {
        let backend = Backend::select(lib_options)?;
        backend.initialize()?;
        let names = backend.names();
        backend.quit();
        names
    }

    /// Open an output device with live process discovery.
    pub fn open(options: &SdlAudioOptions) -> Result<Self> {
        Self::open_with(options, &NativeLibraryOptions::default())
    }

    /// Open an output device with explicit library discovery.
    pub fn open_with(options: &SdlAudioOptions, lib_options: &NativeLibraryOptions) -> Result<Self> {
        options.validate()?;
        let backend = Backend::select(lib_options)?;
        backend.initialize()?;
        let mut port: Option<Box<dyn AudioPort>> = None;
        let result = (|| {
            let opened = backend.open(options)?;
            // Both adapters must expose exact input-byte counts while paused,
            // including resampling tails.
            let channels = options.channels.count() as usize;
            let probe: Vec<u8> = if options.sample_bits == AudioSampleBits::B16 {
                vec![0u8; channels * 8 * 2]
            } else {
                vec![128u8; channels * 8]
            };
            opened.put(&probe)?;
            let queued = opened.queued_bytes()?;
            opened.clear()?;
            if queued != probe.len() as u64 || opened.queued_bytes()? != 0 {
                return Err(Error::unavailable(
                    backend_name(&backend),
                    "SDL audio driver cannot report exact queued input frames at this sample rate",
                ));
            }
            port = Some(opened);
            Ok(())
        })();
        if let Err(error) = result {
            if let Some(port) = port.take() {
                port.close_box();
            }
            backend.quit();
            return Err(error);
        }
        let clock_frequency = backend.frequency();
        if clock_frequency == 0 {
            if let Some(port) = port.take() {
                port.close_box();
            }
            backend.quit();
            return Err(Error::InvalidInput(
                "SDL returned an invalid performance counter frequency".to_string(),
            ));
        }
        let buffer_frames = port.as_ref().map_or(0, |port| port.buffer_frames());
        Ok(Self {
            backend,
            port,
            clock_frequency,
            elapsed_ticks: 0,
            playing_since: None,
            max_queued_frames: u64::from(options.sample_rate) * 2,
            sample_rate: options.sample_rate,
            channels: options.channels,
            sample_bits: options.sample_bits,
            device_name: options.device_name.clone(),
            buffer_frames,
        })
    }

    fn opened(&self) -> Result<&dyn AudioPort> {
        self.port
            .as_deref()
            .ok_or_else(|| Error::Closed("SDL audio device".to_string()))
    }

    /// Sample rate in Hz.
    #[must_use]
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Channel count.
    #[must_use]
    pub fn channels(&self) -> AudioChannels {
        self.channels
    }

    /// Sample width.
    #[must_use]
    pub fn sample_bits(&self) -> AudioSampleBits {
        self.sample_bits
    }

    /// Device name, or `None` for the default.
    #[must_use]
    pub fn device_name(&self) -> Option<&str> {
        self.device_name.as_deref()
    }

    /// Actual buffer duration in input frames.
    #[must_use]
    pub fn buffer_frames(&self) -> u64 {
        self.buffer_frames
    }

    /// Maximum queue depth (two seconds of frames).
    #[must_use]
    pub fn max_queued_frames(&self) -> u64 {
        self.max_queued_frames
    }

    /// Queued input frames.
    pub fn queued_frames(&self) -> Result<u64> {
        let bytes = self.opened()?.queued_bytes()?;
        let frame_bytes = u64::from(self.channels.count()) * u64::from(self.sample_bits.bytes());
        if bytes % frame_bytes != 0 {
            return Err(Error::InvalidInput(
                "SDL queued audio size is not frame aligned".to_string(),
            ));
        }
        Ok(bytes / frame_bytes)
    }

    /// Logical playing time, including empty-queue silence; not a speaker-position measurement.
    pub fn playback_frames(&self) -> Result<u64> {
        self.opened()?;
        let frames = self.playing_ticks()? * u64::from(self.sample_rate) / self.clock_frequency;
        if frames > i64::MAX as u64 {
            return Err(Error::OutOfRange(
                "SDL playback clock exceeds safe frame positions".to_string(),
            ));
        }
        Ok(frames)
    }

    fn playing_ticks(&self) -> Result<u64> {
        let Some(since) = self.playing_since else {
            return Ok(self.elapsed_ticks);
        };
        let now = self.backend.counter();
        if now < since {
            return Err(Error::InvalidInput(
                "SDL performance counter moved backward".to_string(),
            ));
        }
        Ok(self.elapsed_ticks + now - since)
    }

    fn check_queue(&self, frames: u64) -> Result<()> {
        if self.queued_frames()? + frames > self.max_queued_frames {
            return Err(Error::InvalidInput(
                "SDL audio queue exceeds the two-second limit".to_string(),
            ));
        }
        Ok(())
    }

    /// Queue 16-bit samples on a 16-bit device.
    pub fn queue_i16(&mut self, samples: &[i16]) -> Result<()> {
        if self.sample_bits != AudioSampleBits::B16 {
            return Err(Error::InvalidInput(
                "audio samples do not match the device format".to_string(),
            ));
        }
        if !samples.len().is_multiple_of(self.channels.count() as usize) {
            return Err(Error::InvalidInput(
                "audio sample count is not channel aligned".to_string(),
            ));
        }
        let frames = samples.len() as u64 / u64::from(self.channels.count());
        self.check_queue(frames)?;
        if frames != 0 {
            // SAFETY: `i16` samples reinterpret as native-endian bytes.
            let bytes = unsafe { std::slice::from_raw_parts(samples.as_ptr().cast::<u8>(), samples.len() * 2) };
            self.opened()?.put(bytes)?;
        }
        Ok(())
    }

    /// Queue 8-bit samples on an 8-bit device.
    pub fn queue_u8(&mut self, samples: &[u8]) -> Result<()> {
        if self.sample_bits != AudioSampleBits::B8 {
            return Err(Error::InvalidInput(
                "audio samples do not match the device format".to_string(),
            ));
        }
        if !samples.len().is_multiple_of(self.channels.count() as usize) {
            return Err(Error::InvalidInput(
                "audio sample count is not channel aligned".to_string(),
            ));
        }
        let frames = samples.len() as u64 / u64::from(self.channels.count());
        self.check_queue(frames)?;
        if frames != 0 {
            self.opened()?.put(samples)?;
        }
        Ok(())
    }

    /// Drop queued audio.
    pub fn clear(&mut self) -> Result<()> {
        self.opened()?.clear()
    }

    /// Pause playback, freezing the logical clock.
    pub fn pause(&mut self) -> Result<()> {
        if self.port.is_none() {
            return Err(Error::Closed("SDL audio device".to_string()));
        }
        if self.playing_since.is_none() {
            return Ok(());
        }
        let elapsed = self.playing_ticks()?;
        self.opened()?.pause()?;
        self.elapsed_ticks = elapsed;
        self.playing_since = None;
        Ok(())
    }

    /// Resume playback.
    pub fn resume(&mut self) -> Result<()> {
        if self.port.is_none() {
            return Err(Error::Closed("SDL audio device".to_string()));
        }
        if self.playing_since.is_some() {
            return Ok(());
        }
        let now = self.backend.counter();
        self.opened()?.resume()?;
        self.playing_since = Some(now);
        Ok(())
    }

    /// Start playback (alias for [`SdlAudioDevice::resume`]).
    pub fn start(&mut self) -> Result<()> {
        self.resume()
    }

    /// Current playback state.
    #[must_use]
    pub fn state(&self) -> AudioState {
        if self.port.is_none() {
            AudioState::Closed
        } else if self.playing_since.is_none() {
            AudioState::Paused
        } else {
            AudioState::Playing
        }
    }

    /// Close the device and quit the subsystem. Idempotent.
    pub fn close(&mut self) {
        if let Some(port) = self.port.take() {
            port.close_box();
            self.backend.quit();
        }
        self.playing_since = None;
    }

    /// Whether the device is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.port.is_none()
    }
}

impl Drop for SdlAudioDevice {
    fn drop(&mut self) {
        self.close();
    }
}

fn backend_name(backend: &Backend) -> &'static str {
    match backend {
        Backend::Sdl2(_) => "sdl2",
        Backend::Sdl3(_) => "sdl3",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn missing_both() -> NativeLibraryOptions {
        let mut environment = HashMap::new();
        environment.insert(
            "QUAKE_SDL2_LIBRARY".to_string(),
            "/nonexistent-qa-platform/libSDL2.so".to_string(),
        );
        environment.insert(
            "QUAKE_SDL3_LIBRARY".to_string(),
            "/nonexistent-qa-platform/libSDL3.so".to_string(),
        );
        NativeLibraryOptions {
            environment: Some(environment),
            ..NativeLibraryOptions::default()
        }
    }

    #[test]
    fn option_validation() {
        let good = SdlAudioOptions::default();
        assert!(good.validate().is_ok());
        for rate in [7999, 192001] {
            let bad = SdlAudioOptions {
                sample_rate: rate,
                ..good.clone()
            };
            assert!(bad.validate().is_err(), "{rate}");
        }
        let bad = SdlAudioOptions {
            device_name: Some(String::new()),
            ..good.clone()
        };
        assert!(bad.validate().is_err());
        let bad = SdlAudioOptions {
            device_name: Some("bad\0name".to_string()),
            ..good.clone()
        };
        assert!(bad.validate().is_err());
        for frames in [0, 32769] {
            let bad = SdlAudioOptions {
                buffer_frames: frames,
                ..good.clone()
            };
            assert!(bad.validate().is_err(), "{frames}");
        }
    }

    #[test]
    fn sdl2_buffer_rounding() {
        assert_eq!(sdl2_requested_frames(1), 64);
        assert_eq!(sdl2_requested_frames(100), 128);
        assert_eq!(sdl2_requested_frames(1024), 1024);
        assert_eq!(sdl2_requested_frames(32768), 32768);
        assert_eq!(resampled_frames(512, 44100, 48000), 471);
        assert_eq!(resampled_frames(480, 48000, 48000), 480);
    }

    #[test]
    fn missing_libraries_fall_back_then_name_sdl2() {
        // SDL3 missing without an override falls through to SDL2; with both
        // missing, the SDL2 absence names the library.
        let mut environment = HashMap::new();
        environment.insert(
            "QUAKE_SDL2_LIBRARY".to_string(),
            "/nonexistent-qa-platform/libSDL2.so".to_string(),
        );
        environment.insert(
            "QUAKE_SDL3_LIBRARY".to_string(),
            "/nonexistent-qa-platform/libSDL3.so".to_string(),
        );
        let both = NativeLibraryOptions {
            environment: Some(environment),
            ..NativeLibraryOptions::default()
        };
        // Both overrides set: the SDL3 override is authoritative, so the SDL3
        // absence surfaces without trying SDL2.
        let Err(error) = SdlAudioDevice::open_with(&SdlAudioOptions::default(), &both) else {
            panic!("expected failure")
        };
        assert!(error.is_unavailable(), "{error}");
        assert!(error.to_string().contains("sdl3"), "{error}");

        let options = missing_both();
        let Err(error) = SdlAudioDevice::output_device_names_with(&options) else {
            panic!("expected failure")
        };
        assert!(error.is_unavailable(), "{error}");
    }

    #[test]
    fn formats_match_native_endianness() {
        assert_eq!(AudioSampleBits::B16.format(), SIGNED16_NATIVE);
        assert_eq!(AudioSampleBits::B8.format(), UNSIGNED8);
        assert_eq!(AudioSampleBits::B16.bytes(), 2);
        assert_eq!(AudioChannels::Stereo.count(), 2);
    }

    #[test]
    fn live_open_reports_honestly() {
        match SdlAudioDevice::open(&SdlAudioOptions::default()) {
            Ok(mut device) => {
                assert_eq!(device.state(), AudioState::Paused);
                assert!(device.buffer_frames() > 0);
                assert_eq!(device.queued_frames().unwrap(), 0);
                device.start().unwrap();
                assert_eq!(device.state(), AudioState::Playing);
                device.pause().unwrap();
                assert_eq!(device.state(), AudioState::Paused);
                device.close();
                assert!(device.is_closed());
            }
            Err(error) => assert!(!error.to_string().is_empty(), "{error}"),
        }
        match SdlAudioDevice::output_device_names() {
            Ok(_) | Err(_) => {}
        }
    }
}
