//! Device-rate PCM goes through SDL3 streams; mixing stays in qa-audio.
use std::{ffi::c_void, num::NonZeroU32, ptr::NonNull};
#[repr(C)]
struct Spec {
    format: i32,
    channels: i32,
    frequency: i32,
}
#[link(name = "SDL3")]
unsafe extern "C" {
    fn SDL_InitSubSystem(flags: u32) -> bool;
    fn SDL_QuitSubSystem(flags: u32);
    fn SDL_OpenAudioDeviceStream(
        device: u32,
        spec: *const Spec,
        callback: *const c_void,
        userdata: *mut c_void,
    ) -> *mut c_void;
    fn SDL_DestroyAudioStream(stream: *mut c_void);
    fn SDL_ResumeAudioStreamDevice(stream: *mut c_void) -> bool;
    fn SDL_PutAudioStreamData(stream: *mut c_void, data: *const c_void, bytes: i32) -> bool;
    fn SDL_GetAudioStreamQueued(stream: *mut c_void) -> i32;
}
pub struct AudioStream {
    stream: NonNull<c_void>,
    channels: usize,
}
impl AudioStream {
    /// Open at load time. SDL converts device formats; clients share one mixer.
    pub fn open(rate: NonZeroU32, channels: u8) -> Result<Self, String> {
        if !matches!(channels, 1 | 2) || rate.get() > i32::MAX as u32 {
            return Err("invalid audio format".into());
        }
        unsafe {
            if !SDL_InitSubSystem(0x10) {
                return Err(crate::sdl::error());
            }
            let spec = Spec {
                format: if cfg!(target_endian = "little") {
                    0x8010
                } else {
                    0x9010
                },
                channels: i32::from(channels),
                frequency: rate.get() as i32,
            };
            let Some(stream) = NonNull::new(SDL_OpenAudioDeviceStream(
                u32::MAX,
                &spec,
                std::ptr::null(),
                std::ptr::null_mut(),
            )) else {
                let error = crate::sdl::error();
                SDL_QuitSubSystem(0x10);
                return Err(error);
            };
            if !SDL_ResumeAudioStreamDevice(stream.as_ptr()) {
                let error = crate::sdl::error();
                SDL_DestroyAudioStream(stream.as_ptr());
                SDL_QuitSubSystem(0x10);
                return Err(error);
            }
            Ok(Self {
                stream,
                channels: usize::from(channels),
            })
        }
    }
    /// False scopes a device error to this delivery; the host keeps running.
    pub fn write(&mut self, samples: &[i16]) -> bool {
        let bytes = std::mem::size_of_val(samples);
        samples.len().is_multiple_of(self.channels)
            && bytes <= i32::MAX as usize
            && unsafe {
                SDL_PutAudioStreamData(self.stream.as_ptr(), samples.as_ptr().cast(), bytes as i32)
            }
    }
    pub fn queued_bytes(&self) -> Option<usize> {
        usize::try_from(unsafe { SDL_GetAudioStreamQueued(self.stream.as_ptr()) }).ok()
    }
}
impl Drop for AudioStream {
    fn drop(&mut self) {
        unsafe {
            SDL_DestroyAudioStream(self.stream.as_ptr());
            SDL_QuitSubSystem(0x10);
        }
    }
}
