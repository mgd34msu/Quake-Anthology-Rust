//! SDL2 windows, events, joysticks, clipboard, and gamma.
//!
//! Port of donor `src/platform/sdl.ts` (the replacement for
//! `code/unix/linux_glimp.c` window/context duties and `HandleEvents`).
//! Windows are owned by their creating thread; event routing, input, and
//! gamma leases follow the donor's single-owner rules.

use std::collections::HashMap;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use crate::error::{Error, Result};
use crate::ffi_util::{c_string, c_string_bytes, c_string_lossy, sdl_error, LoadedLibrary};
use crate::native_libraries::{host_opengl_driver, NativeLibrary, NativeLibraryOptions};
use crate::sdl_render_context::{
    ProcedureGuard, RenderSdl, SdlRenderContext, SdlRenderContextLease, SdlRenderContextTransfer,
};

const VIDEO_SUBSYSTEM: u32 = 0x20;
const JOYSTICK_SUBSYSTEM: u32 = 0x200;

/// RGBA32 pixel format, matching the donor's endian-dependent constant.
#[cfg(target_endian = "little")]
pub const RGBA32: u32 = 0x1676_2004;
/// RGBA32 pixel format, matching the donor's endian-dependent constant.
#[cfg(target_endian = "big")]
pub const RGBA32: u32 = 0x1646_2004;

macro_rules! sdl2_symbols {
    ($($field:ident : $cname:literal : $sig:ty;)*) => {
        pub(crate) struct Sdl2 {
            _lib: LoadedLibrary,
            $(pub $field: $sig,)*
        }
        impl Sdl2 {
            /// # Safety
            ///
            /// Resolved symbols are only invoked with the SDL ABI below.
            pub unsafe fn load(options: &NativeLibraryOptions) -> Result<Arc<Self>> {
                // SAFETY: loading maps the image without invoking its code.
                let lib = unsafe { LoadedLibrary::open(NativeLibrary::Sdl2, options)? };
                $(let $field: $sig = unsafe { lib.symbol(concat!($cname, "\0").as_bytes())? };)*
                let this = Arc::new(Self { _lib: lib, $($field,)* });
                // SAFETY: no arguments; required before any other SDL call.
                unsafe { (this.sdl_set_main_ready)(); }
                Ok(this)
            }

            /// # Safety
            ///
            /// Every SDL call below upholds its own argument contract.
            unsafe fn error(&self) -> String {
                // SAFETY: SDL guarantees a valid thread-local error string.
                unsafe { sdl_error(self.sdl_get_error) }
            }

            /// # Safety
            ///
            /// `result` must be SDL's return for `operation`.
            unsafe fn checked(&self, result: i32, operation: &str) -> Result<()> {
                if result < 0 {
                    // SAFETY: error read immediately after the failing call.
                    return Err(Error::native(operation, unsafe { self.error() }));
                }
                Ok(())
            }
        }
    };
}

sdl2_symbols! {
    sdl_set_main_ready: "SDL_SetMainReady": unsafe extern "C" fn();
    sdl_init_sub_system: "SDL_InitSubSystem": unsafe extern "C" fn(u32) -> i32;
    sdl_set_hint: "SDL_SetHint": unsafe extern "C" fn(*const u8, *const u8) -> i32;
    sdl_quit_sub_system: "SDL_QuitSubSystem": unsafe extern "C" fn(u32);
    sdl_get_error: "SDL_GetError": unsafe extern "C" fn() -> *const u8;
    sdl_open_url: "SDL_OpenURL": unsafe extern "C" fn(*const u8) -> i32;
    sdl_get_clipboard_text: "SDL_GetClipboardText": unsafe extern "C" fn() -> *mut c_void;
    sdl_set_clipboard_text: "SDL_SetClipboardText": unsafe extern "C" fn(*const u8) -> i32;
    sdl_free: "SDL_free": unsafe extern "C" fn(*mut c_void);
    sdl_show_window: "SDL_ShowWindow": unsafe extern "C" fn(*mut c_void);
    sdl_get_window_position: "SDL_GetWindowPosition": unsafe extern "C" fn(*mut c_void, *mut i32, *mut i32);
    sdl_set_window_position: "SDL_SetWindowPosition": unsafe extern "C" fn(*mut c_void, i32, i32);
    sdl_raise_window: "SDL_RaiseWindow": unsafe extern "C" fn(*mut c_void);
    sdl_maximize_window: "SDL_MaximizeWindow": unsafe extern "C" fn(*mut c_void);
    sdl_minimize_window: "SDL_MinimizeWindow": unsafe extern "C" fn(*mut c_void);
    sdl_restore_window: "SDL_RestoreWindow": unsafe extern "C" fn(*mut c_void);
    sdl_get_window_display_mode: "SDL_GetWindowDisplayMode": unsafe extern "C" fn(*mut c_void, *mut u8) -> i32;
    sdl_hide_window: "SDL_HideWindow": unsafe extern "C" fn(*mut c_void);
    sdl_create_window: "SDL_CreateWindow": unsafe extern "C" fn(*const u8, i32, i32, i32, i32, u32) -> *mut c_void;
    sdl_destroy_window: "SDL_DestroyWindow": unsafe extern "C" fn(*mut c_void);
    sdl_get_window_id: "SDL_GetWindowID": unsafe extern "C" fn(*mut c_void) -> u32;
    sdl_get_window_flags: "SDL_GetWindowFlags": unsafe extern "C" fn(*mut c_void) -> u32;
    sdl_set_window_size: "SDL_SetWindowSize": unsafe extern "C" fn(*mut c_void, i32, i32);
    sdl_get_window_size: "SDL_GetWindowSize": unsafe extern "C" fn(*mut c_void, *mut i32, *mut i32);
    sdl_get_window_size_in_pixels: "SDL_GetWindowSizeInPixels": unsafe extern "C" fn(*mut c_void, *mut i32, *mut i32);
    sdl_set_window_fullscreen: "SDL_SetWindowFullscreen": unsafe extern "C" fn(*mut c_void, u32) -> i32;
    sdl_get_window_display_index: "SDL_GetWindowDisplayIndex": unsafe extern "C" fn(*mut c_void) -> i32;
    sdl_get_num_video_displays: "SDL_GetNumVideoDisplays": unsafe extern "C" fn() -> i32;
    sdl_get_display_bounds: "SDL_GetDisplayBounds": unsafe extern "C" fn(i32, *mut i32) -> i32;
    sdl_get_display_name: "SDL_GetDisplayName": unsafe extern "C" fn(i32) -> *const u8;
    sdl_get_num_display_modes: "SDL_GetNumDisplayModes": unsafe extern "C" fn(i32) -> i32;
    sdl_get_display_mode: "SDL_GetDisplayMode": unsafe extern "C" fn(i32, i32, *mut u8) -> i32;
    sdl_get_current_display_mode: "SDL_GetCurrentDisplayMode": unsafe extern "C" fn(i32, *mut u8) -> i32;
    sdl_get_closest_display_mode: "SDL_GetClosestDisplayMode": unsafe extern "C" fn(i32, *const u8, *mut u8) -> *mut c_void;
    sdl_set_window_display_mode: "SDL_SetWindowDisplayMode": unsafe extern "C" fn(*mut c_void, *const u8) -> i32;
    sdl_get_window_gamma_ramp: "SDL_GetWindowGammaRamp": unsafe extern "C" fn(*mut c_void, *mut u16, *mut u16, *mut u16) -> i32;
    sdl_set_window_gamma_ramp: "SDL_SetWindowGammaRamp": unsafe extern "C" fn(*mut c_void, *const u16, *const u16, *const u16) -> i32;
    sdl_calculate_gamma_ramp: "SDL_CalculateGammaRamp": unsafe extern "C" fn(f32, *mut u16);
    sdl_get_ticks: "SDL_GetTicks": unsafe extern "C" fn() -> u32;
    sdl_set_relative_mouse_mode: "SDL_SetRelativeMouseMode": unsafe extern "C" fn(i32) -> i32;
    sdl_get_relative_mouse_mode: "SDL_GetRelativeMouseMode": unsafe extern "C" fn() -> i32;
    sdl_start_text_input: "SDL_StartTextInput": unsafe extern "C" fn();
    sdl_stop_text_input: "SDL_StopTextInput": unsafe extern "C" fn();
    sdl_num_joysticks: "SDL_NumJoysticks": unsafe extern "C" fn() -> i32;
    sdl_joystick_open: "SDL_JoystickOpen": unsafe extern "C" fn(i32) -> *mut c_void;
    sdl_joystick_close: "SDL_JoystickClose": unsafe extern "C" fn(*mut c_void);
    sdl_joystick_instance_id: "SDL_JoystickInstanceID": unsafe extern "C" fn(*mut c_void) -> i32;
    sdl_joystick_name: "SDL_JoystickName": unsafe extern "C" fn(*mut c_void) -> *const u8;
    sdl_joystick_num_axes: "SDL_JoystickNumAxes": unsafe extern "C" fn(*mut c_void) -> i32;
    sdl_joystick_num_buttons: "SDL_JoystickNumButtons": unsafe extern "C" fn(*mut c_void) -> i32;
    sdl_joystick_num_hats: "SDL_JoystickNumHats": unsafe extern "C" fn(*mut c_void) -> i32;
    sdl_joystick_num_balls: "SDL_JoystickNumBalls": unsafe extern "C" fn(*mut c_void) -> i32;
    sdl_joystick_get_axis: "SDL_JoystickGetAxis": unsafe extern "C" fn(*mut c_void, i32) -> i16;
    sdl_joystick_get_hat: "SDL_JoystickGetHat": unsafe extern "C" fn(*mut c_void, i32) -> u8;
    sdl_joystick_get_button: "SDL_JoystickGetButton": unsafe extern "C" fn(*mut c_void, i32) -> u8;
    sdl_joystick_update: "SDL_JoystickUpdate": unsafe extern "C" fn();
    sdl_create_renderer: "SDL_CreateRenderer": unsafe extern "C" fn(*mut c_void, i32, u32) -> *mut c_void;
    sdl_destroy_renderer: "SDL_DestroyRenderer": unsafe extern "C" fn(*mut c_void);
    sdl_get_renderer_output_size: "SDL_GetRendererOutputSize": unsafe extern "C" fn(*mut c_void, *mut i32, *mut i32) -> i32;
    sdl_create_texture: "SDL_CreateTexture": unsafe extern "C" fn(*mut c_void, u32, i32, i32, i32) -> *mut c_void;
    sdl_destroy_texture: "SDL_DestroyTexture": unsafe extern "C" fn(*mut c_void);
    sdl_set_texture_blend_mode: "SDL_SetTextureBlendMode": unsafe extern "C" fn(*mut c_void, i32) -> i32;
    sdl_update_texture: "SDL_UpdateTexture": unsafe extern "C" fn(*mut c_void, *const c_void, *const u8, i32) -> i32;
    sdl_render_copy: "SDL_RenderCopy": unsafe extern "C" fn(*mut c_void, *mut c_void, *const c_void, *const c_void) -> i32;
    sdl_render_present: "SDL_RenderPresent": unsafe extern "C" fn(*mut c_void);
    sdl_render_read_pixels: "SDL_RenderReadPixels": unsafe extern "C" fn(*mut c_void, *const c_void, u32, *mut u8, i32) -> i32;
    sdl_pump_events: "SDL_PumpEvents": unsafe extern "C" fn();
    sdl_peep_events: "SDL_PeepEvents": unsafe extern "C" fn(*mut u8, i32, i32, u32, u32) -> i32;
    sdl_push_event: "SDL_PushEvent": unsafe extern "C" fn(*const u8) -> i32;
    sdl_event_state: "SDL_EventState": unsafe extern "C" fn(u32, i32) -> u8;
    sdl_gl_set_attribute: "SDL_GL_SetAttribute": unsafe extern "C" fn(i32, i32) -> i32;
    sdl_gl_load_library: "SDL_GL_LoadLibrary": unsafe extern "C" fn(*const c_void) -> i32;
    sdl_gl_unload_library: "SDL_GL_UnloadLibrary": unsafe extern "C" fn();
    sdl_gl_get_attribute: "SDL_GL_GetAttribute": unsafe extern "C" fn(i32, *mut i32) -> i32;
    sdl_gl_create_context: "SDL_GL_CreateContext": unsafe extern "C" fn(*mut c_void) -> *mut c_void;
    sdl_gl_delete_context: "SDL_GL_DeleteContext": unsafe extern "C" fn(*mut c_void);
    sdl_gl_make_current: "SDL_GL_MakeCurrent": unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32;
    sdl_gl_get_drawable_size: "SDL_GL_GetDrawableSize": unsafe extern "C" fn(*mut c_void, *mut i32, *mut i32);
    sdl_gl_get_proc_address: "SDL_GL_GetProcAddress": unsafe extern "C" fn(*const u8) -> *mut c_void;
    sdl_gl_swap_window: "SDL_GL_SwapWindow": unsafe extern "C" fn(*mut c_void);
    sdl_gl_set_swap_interval: "SDL_GL_SetSwapInterval": unsafe extern "C" fn(i32) -> i32;
    sdl_gl_get_swap_interval: "SDL_GL_GetSwapInterval": unsafe extern "C" fn() -> i32;
}

/// Win32 `Sys_GetClipboardData` strtok(data, "\n\r\b") keeps leading delimiters.
pub fn source_clipboard_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut length = 0usize;
    let mut token = false;
    for &byte in bytes {
        if byte == 0 {
            break;
        }
        let delimiter = byte == 10 || byte == 13 || byte == 8;
        if delimiter && token {
            break;
        }
        if !delimiter {
            token = true;
        }
        length += 1;
    }
    let mut result = vec![0u8; length + 1];
    result[..length].copy_from_slice(&bytes[..length]);
    result
}

/// `Sys_DisplayToUse` falls back to the main display for invalid indexes.
pub fn sdl_display_index(requested: i32, count: i32) -> Result<u32> {
    if count <= 0 {
        return Err(Error::InvalidInput("SDL has no video displays".to_string()));
    }
    if requested < 0 || requested >= count {
        return Ok(0);
    }
    Ok(requested as u32)
}

/// A display mode in host order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SdlDisplayMode {
    /// Width in pixels.
    pub width: i32,
    /// Height in pixels.
    pub height: i32,
    /// Color precision in bits.
    pub color_bits: i32,
    /// Refresh rate in Hz.
    pub refresh_rate: i32,
}

/// Exact-match request with refresh limits (0 disables a limit).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SdlDisplayModeRequest {
    /// Width in pixels.
    pub width: i32,
    /// Height in pixels.
    pub height: i32,
    /// Color precision in bits.
    pub color_bits: i32,
    /// Minimum refresh rate, or 0.
    pub min_display_refresh: i32,
    /// Maximum refresh rate, or 0.
    pub max_display_refresh: i32,
}

fn validate_refresh_limits(min: i32, max: i32) -> Result<()> {
    if min != 0 && max != 0 && min > max {
        return Err(Error::InvalidInput(
            "r_minDisplayRefresh must be less than or equal to r_maxDisplayRefresh".to_string(),
        ));
    }
    Ok(())
}

/// `Sys_GetMatchingDisplayMode` retains the last exact matching mode.
pub fn sdl_matching_display_mode(modes: &[SdlDisplayMode], request: &SdlDisplayModeRequest) -> Result<Option<usize>> {
    validate_refresh_limits(request.min_display_refresh, request.max_display_refresh)?;
    let mut selected = None;
    for (index, mode) in modes.iter().enumerate() {
        if mode.width != request.width || mode.height != request.height || mode.color_bits != request.color_bits {
            continue;
        }
        if request.min_display_refresh != 0 && mode.refresh_rate < request.min_display_refresh {
            continue;
        }
        if request.max_display_refresh != 0 && mode.refresh_rate > request.max_display_refresh {
            continue;
        }
        selected = Some(index);
    }
    Ok(selected)
}

/// One GL visual candidate: per-component color bits, depth bits, stencil bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisualAttempt {
    /// Red/green/blue component bits.
    pub component: i32,
    /// Depth bits.
    pub depth: i32,
    /// Stencil bits.
    pub stencil: i32,
}

/// `GLW_SetMode`'s sixteen visual attempts, including the depth/stencil fallthrough.
pub fn visual_attempts(requested_color: i32, requested_depth: i32, requested_stencil: i32) -> Vec<VisualAttempt> {
    fn reduce(bits: i32) -> i32 {
        if bits == 24 {
            16
        } else if bits == 16 {
            8
        } else {
            bits
        }
    }
    let mut color = requested_color;
    let mut depth = requested_depth;
    let mut stencil = requested_stencil;
    let mut out = Vec::with_capacity(16);
    for index in 0..16 {
        if index == 4 {
            depth = reduce(depth);
            stencil = reduce(stencil);
        }
        if index == 8 && color == 24 {
            color = 16;
        }
        if index == 12 {
            stencil = reduce(stencil);
        }
        let candidate_color = if index % 4 == 3 && color == 24 { 16 } else { color };
        out.push(VisualAttempt {
            component: if candidate_color == 24 { 8 } else { 4 },
            depth: if index % 4 == 2 { reduce(depth) } else { depth },
            stencil: if index % 4 == 1 {
                if stencil == 24 {
                    16
                } else if stencil == 16 {
                    8
                } else {
                    0
                }
            } else {
                stencil
            },
        });
    }
    out
}

/// Linux tests the stored cvar float for zero before assigning it to an int.
pub fn source_visual_precision(value: f32) -> Result<i32> {
    if value == 0.0 {
        return Ok(24);
    }
    let integer = value.trunc() as i64;
    if integer < i64::from(i32::MIN) || integer > i64::from(i32::MAX) {
        return Err(Error::OutOfRange("SDL visual precision exceeds int32".to_string()));
    }
    Ok(integer as i32)
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_ne_bytes(bytes[offset..offset + 4].try_into().expect("event field"))
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_ne_bytes(bytes[offset..offset + 4].try_into().expect("event field"))
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_ne_bytes(bytes[offset..offset + 2].try_into().expect("event field"))
}

fn read_i16(bytes: &[u8], offset: usize) -> i16 {
    i16::from_ne_bytes(bytes[offset..offset + 2].try_into().expect("event field"))
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
    f32::from_ne_bytes(bytes[offset..offset + 4].try_into().expect("event field"))
}

/// Decoded SDL window/joystick event.
#[derive(Clone, Debug, PartialEq)]
pub enum SdlEvent {
    /// Application quit requested.
    Quit {
        /// Event timestamp in ms.
        timestamp: u32,
    },
    /// Keyboard key transition.
    Key {
        /// Event timestamp in ms.
        timestamp: u32,
        /// True for press, false for release.
        down: bool,
        /// True for auto-repeat.
        repeat: bool,
        /// SDL scancode.
        scancode: i32,
        /// SDL keycode.
        keycode: i32,
        /// Key modifiers.
        modifiers: u16,
    },
    /// Text input.
    Text {
        /// Event timestamp in ms.
        timestamp: u32,
        /// UTF-8 text.
        text: String,
    },
    /// Mouse motion.
    MouseMotion {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Button mask.
        buttons: u32,
        /// X position.
        x: i32,
        /// Y position.
        y: i32,
        /// Relative X motion.
        dx: i32,
        /// Relative Y motion.
        dy: i32,
    },
    /// Mouse button transition.
    MouseButton {
        /// Event timestamp in ms.
        timestamp: u32,
        /// True for press, false for release.
        down: bool,
        /// Button index.
        button: u8,
        /// Click count.
        clicks: u8,
        /// X position.
        x: i32,
        /// Y position.
        y: i32,
    },
    /// Mouse wheel motion.
    MouseWheel {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Horizontal steps.
        x: i32,
        /// Vertical steps.
        y: i32,
        /// Precise horizontal motion.
        precise_x: f32,
        /// Precise vertical motion.
        precise_y: f32,
        /// Direction flipped.
        flipped: bool,
    },
    /// Window state change.
    Window {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Window event id.
        event: u8,
        /// Event data.
        data1: i32,
        /// Event data.
        data2: i32,
    },
    /// Joystick axis motion.
    JoystickAxis {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Device instance.
        instance: i32,
        /// Axis index.
        axis: u8,
        /// Axis value.
        value: i16,
    },
    /// Joystick hat motion.
    JoystickHat {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Device instance.
        instance: i32,
        /// Hat index.
        hat: u8,
        /// Hat value.
        value: u8,
    },
    /// Joystick button transition.
    JoystickButton {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Device instance.
        instance: i32,
        /// Button index.
        button: u8,
        /// True for press, false for release.
        down: bool,
    },
    /// Joystick removed.
    JoystickRemoved {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Device instance.
        instance: i32,
    },
    /// Any event outside the game input contract.
    Unsupported {
        /// Event timestamp in ms.
        timestamp: u32,
        /// SDL event type.
        event_type: u32,
    },
}

/// Events that may be synthetically injected. Native text/wheel input decodes
/// normally, but sdl2-compat cannot convert injected text safely or wheel
/// steps faithfully, so neither is injectable.
#[derive(Clone, Debug, PartialEq)]
pub enum SdlInjectedEvent {
    /// Application quit requested.
    Quit {
        /// Event timestamp in ms.
        timestamp: u32,
    },
    /// Keyboard key transition.
    Key {
        /// Event timestamp in ms.
        timestamp: u32,
        /// True for press, false for release.
        down: bool,
        /// True for auto-repeat.
        repeat: bool,
        /// SDL scancode.
        scancode: i32,
        /// SDL keycode.
        keycode: i32,
        /// Key modifiers.
        modifiers: u16,
    },
    /// Mouse motion.
    MouseMotion {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Button mask.
        buttons: u32,
        /// X position.
        x: i32,
        /// Y position.
        y: i32,
        /// Relative X motion.
        dx: i32,
        /// Relative Y motion.
        dy: i32,
    },
    /// Mouse button transition.
    MouseButton {
        /// Event timestamp in ms.
        timestamp: u32,
        /// True for press, false for release.
        down: bool,
        /// Button index.
        button: u8,
        /// Click count.
        clicks: u8,
        /// X position.
        x: i32,
        /// Y position.
        y: i32,
    },
    /// Window state change.
    Window {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Window event id.
        event: u8,
        /// Event data.
        data1: i32,
        /// Event data.
        data2: i32,
    },
}

/// Decode one 56-byte SDL event record. Offsets follow SDL2 `SDL_events.h`.
pub fn decode_sdl_event(bytes: &[u8; 56]) -> Result<SdlEvent> {
    let event_type = read_u32(bytes, 0);
    let timestamp = read_u32(bytes, 4);
    match event_type {
        0x100 => Ok(SdlEvent::Quit { timestamp }),
        0x200 => Ok(SdlEvent::Window {
            timestamp,
            event: bytes[12],
            data1: read_i32(bytes, 16),
            data2: read_i32(bytes, 20),
        }),
        0x300 | 0x301 => Ok(SdlEvent::Key {
            timestamp,
            down: event_type == 0x300,
            repeat: bytes[13] != 0,
            scancode: read_i32(bytes, 16),
            keycode: read_i32(bytes, 20),
            modifiers: read_u16(bytes, 24),
        }),
        0x303 => {
            let text_bytes = &bytes[12..44];
            let end = text_bytes.iter().position(|&b| b == 0).unwrap_or(text_bytes.len());
            let text = String::from_utf8(text_bytes[..end].to_vec())
                .map_err(|_| Error::InvalidInput("SDL text event is not valid UTF-8".to_string()))?;
            Ok(SdlEvent::Text { timestamp, text })
        }
        0x400 => Ok(SdlEvent::MouseMotion {
            timestamp,
            buttons: read_u32(bytes, 16),
            x: read_i32(bytes, 20),
            y: read_i32(bytes, 24),
            dx: read_i32(bytes, 28),
            dy: read_i32(bytes, 32),
        }),
        0x401 | 0x402 => Ok(SdlEvent::MouseButton {
            timestamp,
            down: event_type == 0x401,
            button: bytes[16],
            clicks: bytes[18],
            x: read_i32(bytes, 20),
            y: read_i32(bytes, 24),
        }),
        0x403 => Ok(SdlEvent::MouseWheel {
            timestamp,
            x: read_i32(bytes, 16),
            y: read_i32(bytes, 20),
            precise_x: read_f32(bytes, 28),
            precise_y: read_f32(bytes, 32),
            flipped: read_u32(bytes, 24) == 1,
        }),
        0x600 => Ok(SdlEvent::JoystickAxis {
            timestamp,
            instance: read_i32(bytes, 8),
            axis: bytes[12],
            value: read_i16(bytes, 16),
        }),
        0x602 => Ok(SdlEvent::JoystickHat {
            timestamp,
            instance: read_i32(bytes, 8),
            hat: bytes[12],
            value: bytes[13],
        }),
        0x603 | 0x604 => Ok(SdlEvent::JoystickButton {
            timestamp,
            instance: read_i32(bytes, 8),
            button: bytes[12],
            down: event_type == 0x603,
        }),
        0x606 => Ok(SdlEvent::JoystickRemoved {
            timestamp,
            instance: read_i32(bytes, 8),
        }),
        _ => Ok(SdlEvent::Unsupported { timestamp, event_type }),
    }
}

/// Encode an injectable event into a 56-byte SDL record for `window_id`.
fn put_u32(bytes: &mut [u8; 56], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
}

fn put_i32(bytes: &mut [u8; 56], offset: usize, value: i32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
}

/// Encode an injectable event into a 56-byte SDL record for `window_id`.
pub fn encode_sdl_event(event: &SdlInjectedEvent, window_id: u32) -> Result<[u8; 56]> {
    let mut bytes = [0u8; 56];
    let timestamp = match event {
        SdlInjectedEvent::Quit { timestamp }
        | SdlInjectedEvent::Key { timestamp, .. }
        | SdlInjectedEvent::MouseMotion { timestamp, .. }
        | SdlInjectedEvent::MouseButton { timestamp, .. }
        | SdlInjectedEvent::Window { timestamp, .. } => *timestamp,
    };
    put_u32(&mut bytes, 4, timestamp);
    put_u32(&mut bytes, 8, window_id);
    match event {
        SdlInjectedEvent::Quit { .. } => put_u32(&mut bytes, 0, 0x100),
        SdlInjectedEvent::Key {
            down,
            repeat,
            scancode,
            keycode,
            modifiers,
            ..
        } => {
            put_u32(&mut bytes, 0, if *down { 0x300 } else { 0x301 });
            bytes[12] = u8::from(*down);
            bytes[13] = u8::from(*repeat);
            put_i32(&mut bytes, 16, *scancode);
            put_i32(&mut bytes, 20, *keycode);
            bytes[24..26].copy_from_slice(&modifiers.to_ne_bytes());
        }
        SdlInjectedEvent::MouseMotion {
            buttons, x, y, dx, dy, ..
        } => {
            put_u32(&mut bytes, 0, 0x400);
            put_u32(&mut bytes, 16, *buttons);
            put_i32(&mut bytes, 20, *x);
            put_i32(&mut bytes, 24, *y);
            put_i32(&mut bytes, 28, *dx);
            put_i32(&mut bytes, 32, *dy);
        }
        SdlInjectedEvent::MouseButton {
            down,
            button,
            clicks,
            x,
            y,
            ..
        } => {
            put_u32(&mut bytes, 0, if *down { 0x401 } else { 0x402 });
            bytes[16] = *button;
            bytes[17] = u8::from(*down);
            bytes[18] = *clicks;
            put_i32(&mut bytes, 20, *x);
            put_i32(&mut bytes, 24, *y);
        }
        SdlInjectedEvent::Window {
            event, data1, data2, ..
        } => {
            put_u32(&mut bytes, 0, 0x200);
            bytes[12] = *event;
            put_i32(&mut bytes, 16, *data1);
            put_i32(&mut bytes, 20, *data2);
        }
    }
    Ok(bytes)
}

/// Window rendering backend.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum SdlBackend {
    /// Software renderer presenting RGBA frames.
    #[default]
    Cpu,
    /// OpenGL context with source visual selection.
    Gl(SdlGlOptions),
}

/// OpenGL backend options.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SdlGlOptions {
    /// Request stereo visuals.
    pub stereo: bool,
    /// Stencil bits (0..=32).
    pub stencil_bits: i32,
    /// Color precision; 0 selects the source default.
    pub color_bits: f32,
    /// Depth precision; 0 selects the source default.
    pub depth_bits: f32,
    /// System GL driver override; must name the system driver.
    pub driver: Option<String>,
    /// Reject software Mesa renderers when false.
    pub allow_software_gl: bool,
}

/// Window open options.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SdlWindowOptions {
    /// Window title.
    pub title: String,
    /// Width in pixels (1..=16384).
    pub width: i32,
    /// Height in pixels (1..=16384).
    pub height: i32,
    /// Start hidden.
    pub hidden: bool,
    /// Resizable frame.
    pub resizable: bool,
    /// Attempt exclusive fullscreen.
    pub fullscreen: bool,
    /// Preferred display refresh rate.
    pub display_refresh: i32,
    /// Display index; invalid values fall back to the main display.
    pub display_index: i32,
    /// Minimum acceptable refresh rate (0 disables).
    pub min_display_refresh: i32,
    /// Maximum acceptable refresh rate (0 disables).
    pub max_display_refresh: i32,
    /// Position relative to the selected display origin.
    pub position: Option<(i32, i32)>,
    /// Rendering backend.
    pub backend: SdlBackend,
}

/// Captured window presentation for save/restore.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SdlWindowPresentation {
    /// Logical size.
    pub size: (i32, i32),
    /// Position relative to the display origin.
    pub position: (i32, i32),
    /// Display index.
    pub display_index: i32,
    /// Display mode: format, width, height, refresh rate.
    pub display_mode: (u32, i32, i32, i32),
    /// Fullscreen state.
    pub fullscreen: FullscreenMode,
    /// Visible and not hidden.
    pub visible: bool,
    /// Maximized.
    pub maximized: bool,
    /// Minimized.
    pub minimized: bool,
    /// Input focused.
    pub focused: bool,
}

/// Fullscreen state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullscreenMode {
    /// Windowed.
    Windowed,
    /// Borderless desktop fullscreen.
    Desktop,
    /// Exclusive fullscreen.
    Exclusive,
}

/// Display description.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SdlDisplay {
    /// Display index.
    pub index: i32,
    /// Display name.
    pub name: String,
    /// Total display count.
    pub count: i32,
    /// Current refresh rate.
    pub refresh_rate: i32,
    /// Bounds: x, y, width, height.
    pub bounds: (i32, i32, i32, i32),
}

/// Gamma lease capability. Successful set is API acceptance, never hardware readback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SdlGammaCapability {
    /// Gamma calls reach SDL on this display.
    ApiAccepted {
        /// Display index.
        display_index: i32,
        /// Display name.
        display_name: String,
    },
    /// The backend refused gamma.
    Unsupported {
        /// Cause.
        reason: String,
    },
    /// The single-display ownership profile is not met.
    Unavailable {
        /// Cause.
        reason: String,
    },
    /// Display topology changed after acquisition.
    Retired {
        /// Cause.
        reason: String,
    },
}

struct WindowEntry {
    pending: Arc<Mutex<Vec<SdlEvent>>>,
    transfer_state: Mutex<Option<Arc<AtomicI32>>>,
}

fn windows() -> &'static Mutex<HashMap<u32, Arc<WindowEntry>>> {
    static WINDOWS: OnceLock<Mutex<HashMap<u32, Arc<WindowEntry>>>> = OnceLock::new();
    WINDOWS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn input_lease_owner() -> &'static Mutex<Option<u64>> {
    static OWNER: OnceLock<Mutex<Option<u64>>> = OnceLock::new();
    OWNER.get_or_init(|| Mutex::new(None))
}

fn gamma_lease_owner() -> &'static Mutex<Option<u64>> {
    static OWNER: OnceLock<Mutex<Option<u64>>> = OnceLock::new();
    OWNER.get_or_init(|| Mutex::new(None))
}

fn next_lease_id() -> u64 {
    static IDS: AtomicU64 = AtomicU64::new(1);
    IDS.fetch_add(1, Ordering::SeqCst)
}

/// Owned native resources. The creating thread owns their lifetime.
struct Resources {
    kind: ResourceKind,
    window: *mut c_void,
}

enum ResourceKind {
    Cpu {
        renderer: *mut c_void,
        texture: *mut c_void,
        width: i32,
        height: i32,
    },
    Gl {
        context: *mut c_void,
        driver: Option<String>,
    },
}

/// An SDL window. Lifetime belongs to the creating thread.
pub struct SdlWindow {
    sdl: Arc<Sdl2>,
    render_sdl: Option<Arc<RenderSdl>>,
    lib_options: NativeLibraryOptions,
    resources: Option<Resources>,
    id: u32,
    fullscreen_failure: Option<String>,
    position_origin: (i32, i32),
    entry: Arc<WindowEntry>,
    render_lease: Option<SdlRenderContextLease>,
    render_enabled: bool,
    has_frame: bool,
    input: Option<SdlInputLease>,
    gamma: Option<SdlGammaLease>,
    procedure_leases: Arc<std::sync::atomic::AtomicUsize>,
    owner: thread::ThreadId,
    _not_send: PhantomData<*const ()>,
}

#[derive(Clone)]
struct GammaRamp {
    red: [u16; 256],
    green: [u16; 256],
    blue: [u16; 256],
}

impl SdlWindow {
    /// Open a window with live process discovery.
    pub fn open(options: &SdlWindowOptions) -> Result<Self> {
        Self::open_with(options, &NativeLibraryOptions::default())
    }

    /// Open a window with explicit library discovery (tests inject overrides).
    pub fn open_with(options: &SdlWindowOptions, lib_options: &NativeLibraryOptions) -> Result<Self> {
        let owner = thread::current().id();
        Self::require_window_list_ownership()?;
        Self::validate_options(options)?;
        let gl_options = match &options.backend {
            SdlBackend::Gl(gl) => Some(gl.clone()),
            SdlBackend::Cpu => None,
        };
        if let Some(driver) = gl_options.as_ref().and_then(|gl| gl.driver.as_deref()) {
            if driver.is_empty() {
                return Err(Error::InvalidInput("SDL GL driver name is empty".to_string()));
            }
            // Only the system GL library is supported, not legacy vendor drivers.
            let system = host_opengl_driver()?;
            if driver.to_lowercase() != system.to_lowercase() {
                return Err(Error::Unsupported(format!(
                    "unsupported SDL system OpenGL driver: {driver}"
                )));
            }
        }
        // SAFETY: loading maps the image; SDL calls below use validated arguments.
        let sdl = unsafe { Sdl2::load(lib_options)? };
        // SAFETY: SDL calls below use validated arguments.
        unsafe {
            Self::initialize_subsystem(&sdl)?;
            match Self::create_window(&sdl, options, gl_options.as_ref(), owner, lib_options) {
                Ok(window) => Ok(window),
                Err(error) => {
                    (sdl.sdl_quit_sub_system)(VIDEO_SUBSYSTEM);
                    Err(error)
                }
            }
        }
    }

    fn validate_options(options: &SdlWindowOptions) -> Result<()> {
        for dimension in [options.width, options.height] {
            if dimension <= 0 || dimension > 16384 {
                return Err(Error::OutOfRange(
                    "SDL window dimensions must be integers in 1..16384".to_string(),
                ));
            }
        }
        validate_refresh_limits(options.min_display_refresh, options.max_display_refresh)?;
        if let SdlBackend::Gl(gl) = &options.backend {
            if gl.stencil_bits < 0 || gl.stencil_bits > 32 {
                return Err(Error::OutOfRange(
                    "SDL stencil precision must be an integer in 0..32".to_string(),
                ));
            }
            source_visual_precision(gl.color_bits)?;
            source_visual_precision(gl.depth_bits)?;
        }
        Ok(())
    }

    /// # Safety
    ///
    /// The SDL handle must be fully loaded.
    unsafe fn initialize_subsystem(sdl: &Sdl2) -> Result<()> {
        // SAFETY: hint buffers are live for the call.
        unsafe {
            let key = c_string("SDL_NO_SIGNAL_HANDLERS")?;
            let value = c_string("1")?;
            if (sdl.sdl_set_hint)(key.as_ptr(), value.as_ptr()) != 1 {
                return Err(Error::InvalidInput(
                    "SDL must leave signal handling to the Unix signal owner".to_string(),
                ));
            }
            sdl.checked((sdl.sdl_init_sub_system)(VIDEO_SUBSYSTEM), "SDL_InitSubSystem")?;
        }
        Ok(())
    }

    /// # Safety
    ///
    /// The SDL handle must be fully loaded; the caller quits the subsystem on error.
    unsafe fn create_window(
        sdl: &Arc<Sdl2>,
        options: &SdlWindowOptions,
        gl: Option<&SdlGlOptions>,
        owner: thread::ThreadId,
        lib_options: &NativeLibraryOptions,
    ) -> Result<Self> {
        // SAFETY: all SDL calls below use validated arguments; failures unwind
        // through the cleanup path at the end of this function.
        unsafe {
            let display_count = (sdl.sdl_get_num_video_displays)();
            sdl.checked(display_count, "SDL_GetNumVideoDisplays")?;
            let display_index = sdl_display_index(options.display_index, display_count)?;
            let mut bounds = [0i32; 4];
            sdl.checked(
                (sdl.sdl_get_display_bounds)(display_index as i32, bounds.as_mut_ptr()),
                "SDL_GetDisplayBounds",
            )?;
            let position_origin = (bounds[0], bounds[1]);
            let centered = 0x1fff_0000 | display_index;
            let (x, y) = if options.fullscreen {
                (centered as i32, centered as i32)
            } else if let Some((px, py)) = options.position {
                (bounds[0].saturating_add(px), bounds[1].saturating_add(py))
            } else {
                (centered as i32, centered as i32)
            };
            // These event types own native strings and are outside the game input contract.
            (sdl.sdl_event_state)(0x1000, 0);
            (sdl.sdl_event_state)(0x1001, 0);
            (sdl.sdl_event_state)(0x305, 0);
            let flags = (if options.hidden { 0x8 } else { 0x4 })
                | (if gl.is_some() { 0x2 } else { 0 })
                | (if options.resizable { 0x20 } else { 0 });
            let title = c_string(&options.title)?;
            let mut window: *mut c_void = std::ptr::null_mut();
            let mut renderer: *mut c_void = std::ptr::null_mut();
            let mut texture: *mut c_void = std::ptr::null_mut();
            let mut context: *mut c_void = std::ptr::null_mut();
            let mut driver_loaded = false;
            let result = Self::create_window_inner(
                sdl,
                options,
                gl,
                title.as_ptr(),
                x,
                y,
                flags,
                &mut window,
                &mut renderer,
                &mut texture,
                &mut context,
                &mut driver_loaded,
            );
            if let Err(error) = result {
                if !texture.is_null() {
                    (sdl.sdl_destroy_texture)(texture);
                }
                if !renderer.is_null() {
                    (sdl.sdl_destroy_renderer)(renderer);
                }
                if !context.is_null() {
                    (sdl.sdl_gl_delete_context)(context);
                }
                if !window.is_null() {
                    (sdl.sdl_destroy_window)(window);
                }
                if driver_loaded {
                    (sdl.sdl_gl_unload_library)();
                }
                return Err(error);
            }
            let (resources, fullscreen_failure) = result.expect("checked above");
            let id = (sdl.sdl_get_window_id)(window);
            if id == 0 {
                if let ResourceKind::Cpu { renderer, texture, .. } = resources {
                    (sdl.sdl_destroy_texture)(texture);
                    (sdl.sdl_destroy_renderer)(renderer);
                } else if let ResourceKind::Gl { context, .. } = resources {
                    (sdl.sdl_gl_delete_context)(context);
                }
                (sdl.sdl_destroy_window)(window);
                if driver_loaded {
                    (sdl.sdl_gl_unload_library)();
                }
                return Err(Error::native("SDL_GetWindowID", sdl.error()));
            }
            let entry = Arc::new(WindowEntry {
                pending: Arc::new(Mutex::new(Vec::new())),
                transfer_state: Mutex::new(None),
            });
            windows().lock().expect("window map").insert(id, Arc::clone(&entry));
            Ok(Self {
                sdl: Arc::clone(sdl),
                render_sdl: None,
                lib_options: lib_options.clone(),
                resources: Some(Resources {
                    kind: resources,
                    window,
                }),
                id,
                fullscreen_failure,
                position_origin,
                entry,
                render_lease: None,
                render_enabled: true,
                has_frame: false,
                input: None,
                gamma: None,
                procedure_leases: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                owner,
                _not_send: PhantomData,
            })
        }
    }

    /// # Safety
    ///
    /// Out-pointers must be exclusive; the caller destroys partial resources on error.
    #[allow(clippy::too_many_arguments)]
    unsafe fn create_window_inner(
        sdl: &Arc<Sdl2>,
        options: &SdlWindowOptions,
        gl: Option<&SdlGlOptions>,
        title: *const u8,
        x: i32,
        y: i32,
        flags: u32,
        window: &mut *mut c_void,
        renderer: &mut *mut c_void,
        texture: &mut *mut c_void,
        context: &mut *mut c_void,
        driver_loaded: &mut bool,
    ) -> Result<(ResourceKind, Option<String>)> {
        // SAFETY: caller guarantees exclusive out-pointers and cleanup on error.
        unsafe {
            if let Some(gl) = gl {
                let color_bits = source_visual_precision(gl.color_bits)?;
                let depth_bits = source_visual_precision(gl.depth_bits)?;
                // Only the system driver is accepted, so concurrent GL windows
                // necessarily share it; pass null when one already loaded it.
                let sharing = !windows().lock().expect("window map").is_empty();
                if let Some(driver) = &gl.driver {
                    let name = c_string(driver)?;
                    let path = if sharing {
                        name.as_ptr().cast()
                    } else {
                        std::ptr::null()
                    };
                    sdl.checked((sdl.sdl_gl_load_library)(path), "SDL_GL_LoadLibrary")?;
                    *driver_loaded = true;
                }
                sdl.checked((sdl.sdl_gl_set_attribute)(17, 2), "SDL_GL_CONTEXT_MAJOR_VERSION")?;
                sdl.checked((sdl.sdl_gl_set_attribute)(18, 1), "SDL_GL_CONTEXT_MINOR_VERSION")?;
                sdl.checked(
                    (sdl.sdl_gl_set_attribute)(21, 2),
                    "SDL_GL_CONTEXT_PROFILE_COMPATIBILITY",
                )?;
                sdl.checked((sdl.sdl_gl_set_attribute)(5, 1), "SDL_GL_DOUBLEBUFFER")?;
                // Portable renderer stereo support; a mono context must request zero
                // because SDL retains attributes across windows.
                sdl.checked((sdl.sdl_gl_set_attribute)(12, i32::from(gl.stereo)), "SDL_GL_STEREO")?;
                sdl.checked((sdl.sdl_gl_set_attribute)(3, 0), "SDL_GL_ALPHA_SIZE")?;
                let mut failures = Vec::new();
                for visual in visual_attempts(color_bits, depth_bits, gl.stencil_bits) {
                    let attempt = Self::try_visual(sdl, title, x, y, options.width, options.height, flags, &visual);
                    match attempt {
                        Ok((w, c)) => {
                            *window = w;
                            *context = c;
                            break;
                        }
                        Err(error) => failures.push(error),
                    }
                }
                if window.is_null() || context.is_null() {
                    return Err(Error::aggregate(
                        "SDL could not create any source GL visual candidate",
                        failures,
                    ));
                }
                if !gl.allow_software_gl {
                    let name = c_string("glGetString")?;
                    let address = (sdl.sdl_gl_get_proc_address)(name.as_ptr());
                    if address.is_null() {
                        return Err(Error::native("SDL_GL_GetProcAddress glGetString", sdl.error()));
                    }
                    let query: unsafe extern "C" fn(u32) -> *const u8 = std::mem::transmute(address);
                    let renderer_name = c_string_lossy(query(0x1f01)).to_lowercase();
                    // GLW_SetMode rejects only these two source tokens, not modern Mesa drivers.
                    if renderer_name == "mesa x11" || renderer_name == "mesa glx indirect" {
                        return Err(Error::Unsupported(
                            "you are using software Mesa; add +set r_allowSoftwareGL 1 to allow this driver"
                                .to_string(),
                        ));
                    }
                }
            } else {
                *window = (sdl.sdl_create_window)(title, x, y, options.width, options.height, flags);
                if window.is_null() {
                    return Err(Error::native("SDL_CreateWindow", sdl.error()));
                }
            }
            let fullscreen_failure = Self::enter_fullscreen(sdl, *window, options)?;
            if gl.is_none() {
                *renderer = (sdl.sdl_create_renderer)(*window, -1, 1);
                if renderer.is_null() {
                    return Err(Error::native("SDL_CreateRenderer SOFTWARE", sdl.error()));
                }
                let mut width = 0i32;
                let mut height = 0i32;
                sdl.checked(
                    (sdl.sdl_get_renderer_output_size)(*renderer, &mut width, &mut height),
                    "SDL_GetRendererOutputSize",
                )?;
                if width <= 0 || height <= 0 {
                    return Err(Error::native(
                        "SDL_GetRendererOutputSize",
                        "SDL returned invalid drawable dimensions".to_string(),
                    ));
                }
                *texture = (sdl.sdl_create_texture)(*renderer, RGBA32, 1, width, height);
                if texture.is_null() {
                    return Err(Error::native("SDL_CreateTexture", sdl.error()));
                }
                sdl.checked(
                    (sdl.sdl_set_texture_blend_mode)(*texture, 0),
                    "SDL_SetTextureBlendMode NONE",
                )?;
                Ok((
                    ResourceKind::Cpu {
                        renderer: *renderer,
                        texture: *texture,
                        width,
                        height,
                    },
                    fullscreen_failure,
                ))
            } else {
                let mut width = 0i32;
                let mut height = 0i32;
                (sdl.sdl_gl_get_drawable_size)(*window, &mut width, &mut height);
                if width <= 0 || height <= 0 {
                    return Err(Error::native(
                        "SDL_GL_GetDrawableSize",
                        "SDL returned invalid drawable dimensions".to_string(),
                    ));
                }
                Ok((
                    ResourceKind::Gl {
                        context: *context,
                        driver: gl.and_then(|gl| gl.driver.clone()),
                    },
                    fullscreen_failure,
                ))
            }
        }
    }

    /// # Safety
    ///
    /// The SDL handle must be loaded; the returned handles are live on success.
    #[allow(clippy::too_many_arguments)]
    unsafe fn try_visual(
        sdl: &Arc<Sdl2>,
        title: *const u8,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        flags: u32,
        visual: &VisualAttempt,
    ) -> Result<(*mut c_void, *mut c_void)> {
        // SAFETY: SDL calls use validated arguments; partial handles are destroyed.
        unsafe {
            for attribute in [0, 1, 2] {
                sdl.checked(
                    (sdl.sdl_gl_set_attribute)(attribute, visual.component),
                    "SDL_GL RGB_SIZE",
                )?;
            }
            sdl.checked((sdl.sdl_gl_set_attribute)(6, visual.depth), "SDL_GL_DEPTH_SIZE")?;
            sdl.checked((sdl.sdl_gl_set_attribute)(7, visual.stencil), "SDL_GL_STENCIL_SIZE")?;
            let window = (sdl.sdl_create_window)(title, x, y, width, height, flags);
            if window.is_null() {
                return Err(Error::native("SDL_CreateWindow", sdl.error()));
            }
            let context = (sdl.sdl_gl_create_context)(window);
            if context.is_null() {
                (sdl.sdl_destroy_window)(window);
                return Err(Error::native("SDL_GL_CreateContext", sdl.error()));
            }
            let outcome: Result<()> = (|| {
                sdl.checked((sdl.sdl_gl_make_current)(window, context), "SDL_GL_MakeCurrent")?;
                let attributes: [(i32, i32); 6] = [
                    (0, visual.component),
                    (1, visual.component),
                    (2, visual.component),
                    (3, 0),
                    (6, visual.depth),
                    (7, visual.stencil),
                ];
                for (attribute, minimum) in attributes {
                    let mut actual = 0i32;
                    sdl.checked(
                        (sdl.sdl_gl_get_attribute)(attribute, &mut actual),
                        "SDL_GL_GetAttribute",
                    )?;
                    if actual < 0 || actual < minimum {
                        return Err(Error::native(
                            "SDL_GL_GetAttribute",
                            format!("SDL visual attribute {attribute} did not satisfy {minimum} bits"),
                        ));
                    }
                }
                Ok(())
            })();
            match outcome {
                Ok(()) => Ok((window, context)),
                Err(error) => {
                    (sdl.sdl_gl_delete_context)(context);
                    (sdl.sdl_destroy_window)(window);
                    Err(error)
                }
            }
        }
    }

    /// # Safety
    ///
    /// `window` must be a live SDL window.
    unsafe fn enter_fullscreen(
        sdl: &Arc<Sdl2>,
        window: *mut c_void,
        options: &SdlWindowOptions,
    ) -> Result<Option<String>> {
        if !options.fullscreen {
            return Ok(None);
        }
        // SAFETY: the window handle is live.
        unsafe {
            let refresh = options.display_refresh;
            let index = (sdl.sdl_get_window_display_index)(window);
            sdl.checked(index, "SDL_GetWindowDisplayIndex")?;
            // SDL2 SDL_DisplayMode: four 32-bit fields and an aligned driver pointer.
            let mut requested = [0u8; 24];
            let mut closest = [0u8; 24];
            requested[4..8].copy_from_slice(&options.width.to_ne_bytes());
            requested[8..12].copy_from_slice(&options.height.to_ne_bytes());
            requested[12..16].copy_from_slice(&refresh.to_ne_bytes());
            let min = options.min_display_refresh;
            let max = options.max_display_refresh;
            let found = if min != 0 || max != 0 {
                let count = (sdl.sdl_get_num_display_modes)(index);
                sdl.checked(count, "SDL_GetNumDisplayModes")?;
                let mut desktop = [0u8; 24];
                sdl.checked(
                    (sdl.sdl_get_current_display_mode)(index, desktop.as_mut_ptr()),
                    "SDL_GetCurrentDisplayMode",
                )?;
                let requested_color = match &options.backend {
                    SdlBackend::Gl(gl) => gl.color_bits.trunc() as i32,
                    SdlBackend::Cpu => 0,
                };
                let desktop_mode = display_mode_from_bytes(&desktop);
                let color_bits = if requested_color < 16 {
                    desktop_mode.color_bits
                } else {
                    requested_color
                };
                let mut natives = Vec::new();
                let mut modes = Vec::new();
                for ordinal in 0..count {
                    let mut bytes = [0u8; 24];
                    sdl.checked(
                        (sdl.sdl_get_display_mode)(index, ordinal, bytes.as_mut_ptr()),
                        "SDL_GetDisplayMode",
                    )?;
                    natives.push(bytes);
                    modes.push(display_mode_from_bytes(&bytes));
                }
                let selected = sdl_matching_display_mode(
                    &modes,
                    &SdlDisplayModeRequest {
                        width: options.width,
                        height: options.height,
                        color_bits,
                        min_display_refresh: min,
                        max_display_refresh: max,
                    },
                )?;
                match selected {
                    Some(ordinal) => {
                        closest.copy_from_slice(&natives[ordinal]);
                        true
                    }
                    None => false,
                }
            } else {
                !(sdl.sdl_get_closest_display_mode)(index, requested.as_ptr(), closest.as_mut_ptr()).is_null()
            };
            if !found {
                // GLW_SetMode continues windowed when no mode fits, leaving the cvars alone.
                let failure = if min != 0 || max != 0 {
                    "No suitable display mode available within the refresh limits.".to_string()
                } else {
                    format!("SDL_GetClosestDisplayMode: {}", sdl.error())
                };
                if (sdl.sdl_get_window_flags)(window) & 1 != 0 {
                    return Err(Error::native(
                        "SDL_GetClosestDisplayMode",
                        format!("{failure}; SDL's newly created window is unexpectedly fullscreen"),
                    ));
                }
                return Ok(Some(failure));
            }
            let failure: String = if (sdl.sdl_set_window_display_mode)(window, closest.as_ptr()) < 0 {
                format!("SDL_SetWindowDisplayMode: {}", sdl.error())
            } else if (sdl.sdl_set_window_fullscreen)(window, 1) < 0 {
                format!("SDL_SetWindowFullscreen: {}", sdl.error())
            } else if (sdl.sdl_get_window_flags)(window) & 1 != 0 {
                return Ok(None);
            } else {
                "SDL did not enter the requested fullscreen mode".to_string()
            };
            // SDL transition failures have no XF86 counterpart. Continue only after
            // restoring the actual window; failed restoration stays fatal.
            sdl.checked(
                (sdl.sdl_set_window_fullscreen)(window, 0),
                &format!("SDL_SetWindowFullscreen windowed fallback after {failure}"),
            )?;
            if (sdl.sdl_get_window_flags)(window) & 1 != 0 {
                return Err(Error::native(
                    "SDL_SetWindowFullscreen",
                    format!("{failure}; SDL did not restore windowed mode"),
                ));
            }
            Ok(Some(failure))
        }
    }
}

fn display_mode_from_bytes(bytes: &[u8; 24]) -> SdlDisplayMode {
    let format = read_u32(bytes, 0);
    SdlDisplayMode {
        color_bits: ((format >> 8) & 255) as i32,
        width: read_i32(bytes, 4),
        height: read_i32(bytes, 8),
        refresh_rate: read_i32(bytes, 12),
    }
}

/// Whether a captured presentation carries a display mode worth restoring.
///
/// Windowed captures record the zeroed sentinel when SDL reports no mode
/// (see [`SdlWindow::capture_presentation`]); the mode set only affects
/// later fullscreen entry, so restore skips it for those captures.
fn presentation_has_display_mode(state: &SdlWindowPresentation) -> bool {
    state.display_mode != (0, 0, 0, 0)
}

/// Query the display hosting `window`.
unsafe fn query_display(sdl: &Sdl2, window: *mut c_void) -> Result<SdlDisplay> {
    // SAFETY: the window handle is live.
    unsafe {
        let index = (sdl.sdl_get_window_display_index)(window);
        sdl.checked(index, "SDL_GetWindowDisplayIndex")?;
        let count = (sdl.sdl_get_num_video_displays)();
        sdl.checked(count, "SDL_GetNumVideoDisplays")?;
        // SDL_DisplayMode has four 32-bit fields followed by a pointer; only refresh_rate is read.
        let mut mode = [0u8; 24];
        sdl.checked(
            (sdl.sdl_get_current_display_mode)(index, mode.as_mut_ptr()),
            "SDL_GetCurrentDisplayMode",
        )?;
        let mut bounds = [0i32; 4];
        sdl.checked(
            (sdl.sdl_get_display_bounds)(index, bounds.as_mut_ptr()),
            "SDL_GetDisplayBounds",
        )?;
        if bounds[2] <= 0 || bounds[3] <= 0 {
            return Err(Error::native(
                "SDL_GetDisplayBounds",
                "SDL returned invalid display bounds".to_string(),
            ));
        }
        Ok(SdlDisplay {
            index,
            name: c_string_lossy((sdl.sdl_get_display_name)(index)),
            count,
            refresh_rate: read_i32(&mode, 12),
            bounds: (bounds[0], bounds[1], bounds[2], bounds[3]),
        })
    }
}

impl SdlWindow {
    fn require_window_list_ownership() -> Result<()> {
        let map = windows().lock().expect("window map");
        for entry in map.values() {
            let state = entry.transfer_state.lock().expect("transfer state").clone();
            if let Some(state) = state {
                if state.load(Ordering::SeqCst) != crate::sdl_render_context::TRANSFER_PARKED {
                    return Err(Error::InvalidInput(
                        "SDL window list is reserved for the render worker".to_string(),
                    ));
                }
            }
        }
        Ok(())
    }

    fn check_owner(&self) -> Result<()> {
        if thread::current().id() != self.owner {
            return Err(Error::InvalidInput(
                "SDL window lifetime belongs to the owning thread".to_string(),
            ));
        }
        Ok(())
    }

    fn opened(&self) -> Result<&Resources> {
        self.check_owner()?;
        self.resources
            .as_ref()
            .ok_or_else(|| Error::Closed("SDL window".to_string()))
    }

    fn render_reserved(&self) -> bool {
        self.render_lease.as_ref().is_some_and(|lease| !lease.parked())
    }

    /// SDL window id.
    #[must_use]
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Fullscreen failure that fell back to windowed, if any.
    #[must_use]
    pub fn fullscreen_failure(&self) -> Option<&str> {
        self.fullscreen_failure.as_deref()
    }

    /// Selected display origin at open time.
    #[must_use]
    pub fn position_origin(&self) -> (i32, i32) {
        self.position_origin
    }

    /// Raw window flags.
    pub fn flags(&self) -> Result<u32> {
        let window = self.opened()?.window;
        // SAFETY: the window handle is live.
        Ok(unsafe { (self.sdl.sdl_get_window_flags)(window) })
    }

    /// SDL tick count in ms.
    pub fn ticks(&self) -> Result<u32> {
        self.opened()?;
        // SAFETY: no arguments.
        Ok(unsafe { (self.sdl.sdl_get_ticks)() })
    }

    /// Whether the window is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.resources.is_none()
    }

    /// Whether relative mouse mode is held by this window's input lease.
    #[must_use]
    pub fn relative_mouse(&self) -> bool {
        if self.resources.is_none() {
            return false;
        }
        let Some(lease) = &self.input else {
            return false;
        };
        if lease.is_closed() {
            return false;
        }
        // SAFETY: no arguments.
        unsafe { (self.sdl.sdl_get_relative_mouse_mode)() != 0 }
    }

    /// Rendering backend.
    pub fn backend(&self) -> Result<SdlBackendKind> {
        Ok(match &self.opened()?.kind {
            ResourceKind::Cpu { .. } => SdlBackendKind::Cpu,
            ResourceKind::Gl { .. } => SdlBackendKind::Gl,
        })
    }

    /// Drawable width in pixels.
    pub fn width(&self) -> Result<i32> {
        Ok(self.drawable_size_signed()?.0)
    }

    /// Drawable height in pixels.
    pub fn height(&self) -> Result<i32> {
        Ok(self.drawable_size_signed()?.1)
    }

    /// Logical window size.
    pub fn logical_size(&self) -> Result<(i32, i32)> {
        let window = self.opened()?.window;
        let mut width = 0i32;
        let mut height = 0i32;
        // SAFETY: out-pointers describe live integers.
        unsafe {
            (self.sdl.sdl_get_window_size)(window, &mut width, &mut height);
        }
        if width <= 0 || height <= 0 {
            return Err(Error::native(
                "SDL_GetWindowSize",
                "SDL returned invalid window dimensions".to_string(),
            ));
        }
        Ok((width, height))
    }

    /// Drawable size in pixels.
    pub fn drawable_size_signed(&self) -> Result<(i32, i32)> {
        let resources = self.opened()?;
        let mut width = 0i32;
        let mut height = 0i32;
        // SAFETY: the window handle is live; out-pointers describe live integers.
        unsafe {
            match resources.kind {
                // SDL2-compat's software renderer caches its surface until
                // presentation after a resize.
                ResourceKind::Cpu { .. } => {
                    (self.sdl.sdl_get_window_size_in_pixels)(resources.window, &mut width, &mut height);
                }
                ResourceKind::Gl { .. } => {
                    (self.sdl.sdl_gl_get_drawable_size)(resources.window, &mut width, &mut height);
                }
            }
        }
        if width <= 0 || height <= 0 {
            return Err(Error::native(
                "SDL drawable size",
                "SDL returned invalid drawable dimensions".to_string(),
            ));
        }
        Ok((width, height))
    }

    /// Display hosting the window.
    pub fn display(&self) -> Result<SdlDisplay> {
        let window = self.opened()?.window;
        // SAFETY: the window handle is live.
        unsafe { query_display(&self.sdl, window) }
    }

    /// All display modes plus the current desktop mode.
    pub fn display_modes(&self) -> Result<Vec<SdlDisplayMode>> {
        let display = self.display()?;
        let mut modes = Vec::new();
        // SAFETY: out-buffers are live for each call.
        unsafe {
            let count = (self.sdl.sdl_get_num_display_modes)(display.index);
            self.sdl.checked(count, "SDL_GetNumDisplayModes")?;
            for ordinal in 0..count {
                let mut bytes = [0u8; 24];
                self.sdl.checked(
                    (self.sdl.sdl_get_display_mode)(display.index, ordinal, bytes.as_mut_ptr()),
                    "SDL_GetDisplayMode",
                )?;
                modes.push(display_mode_from_bytes(&bytes));
            }
            let mut desktop = [0u8; 24];
            self.sdl.checked(
                (self.sdl.sdl_get_current_display_mode)(display.index, desktop.as_mut_ptr()),
                "SDL_GetCurrentDisplayMode",
            )?;
            modes.push(display_mode_from_bytes(&desktop));
        }
        Ok(modes)
    }

    /// Show or hide the window.
    pub fn set_visible(&self, visible: bool) -> Result<()> {
        let window = self.opened()?.window;
        // SAFETY: the window handle is live.
        unsafe {
            if visible {
                (self.sdl.sdl_show_window)(window);
            } else {
                (self.sdl.sdl_hide_window)(window);
            }
        }
        Ok(())
    }

    /// Capture the current presentation for later restore.
    pub fn capture_presentation(&self) -> Result<SdlWindowPresentation> {
        let resources = self.opened()?;
        let window = resources.window;
        let flags = self.flags()?;
        let display = self.display()?;
        let fullscreen = if flags & 0x1001 == 0x1001 {
            FullscreenMode::Desktop
        } else if flags & 1 != 0 {
            FullscreenMode::Exclusive
        } else {
            FullscreenMode::Windowed
        };
        let mut x = 0i32;
        let mut y = 0i32;
        let mut bytes = [0u8; 24];
        // SAFETY: the window handle is live; out-pointers describe live data.
        let mode_status = unsafe {
            (self.sdl.sdl_get_window_position)(window, &mut x, &mut y);
            (self.sdl.sdl_get_window_display_mode)(window, bytes.as_mut_ptr())
        };
        if mode_status < 0 && fullscreen != FullscreenMode::Windowed {
            // SAFETY: the status came from the live SDL handle above.
            unsafe {
                self.sdl.checked(mode_status, "SDL_GetWindowDisplayMode")?;
            }
        }
        let display_mode = if mode_status >= 0 {
            (
                read_u32(&bytes, 0),
                read_i32(&bytes, 4),
                read_i32(&bytes, 8),
                read_i32(&bytes, 12),
            )
        } else {
            // Windowed windows carry no display mode: SDL reports no match
            // when no listed mode fits (e.g. a window larger than a default
            // Xvfb screen). Record the zeroed sentinel; restore skips the
            // mode set for it.
            debug_assert_eq!(fullscreen, FullscreenMode::Windowed);
            (0, 0, 0, 0)
        };
        Ok(SdlWindowPresentation {
            size: self.logical_size()?,
            position: (x - display.bounds.0, y - display.bounds.1),
            display_index: display.index,
            display_mode,
            fullscreen,
            visible: flags & 4 != 0 && flags & 8 == 0,
            maximized: flags & 0x80 != 0,
            minimized: flags & 0x40 != 0,
            focused: flags & 0x200 != 0,
        })
    }

    /// Restore a captured presentation.
    pub fn restore_presentation(&mut self, state: &SdlWindowPresentation) -> Result<()> {
        let window = self.opened()?.window;
        // SAFETY: the window handle is live; out-pointers describe live data.
        unsafe {
            let mut bounds = [0i32; 4];
            self.sdl.checked(
                (self.sdl.sdl_get_display_bounds)(state.display_index, bounds.as_mut_ptr()),
                "SDL_GetDisplayBounds",
            )?;
            (self.sdl.sdl_restore_window)(window);
            self.sdl.checked(
                (self.sdl.sdl_set_window_fullscreen)(window, 0),
                "SDL_SetWindowFullscreen windowed",
            )?;
            (self.sdl.sdl_set_window_position)(window, bounds[0] + state.position.0, bounds[1] + state.position.1);
        }
        self.set_size(state.size.0, state.size.1)?;
        // SAFETY: the window handle is live.
        unsafe {
            if presentation_has_display_mode(state) {
                let mut bytes = [0u8; 24];
                bytes[0..4].copy_from_slice(&state.display_mode.0.to_ne_bytes());
                bytes[4..8].copy_from_slice(&state.display_mode.1.to_ne_bytes());
                bytes[8..12].copy_from_slice(&state.display_mode.2.to_ne_bytes());
                bytes[12..16].copy_from_slice(&state.display_mode.3.to_ne_bytes());
                self.sdl.checked(
                    (self.sdl.sdl_set_window_display_mode)(window, bytes.as_ptr()),
                    "SDL_SetWindowDisplayMode",
                )?;
            }
            let mode = match state.fullscreen {
                FullscreenMode::Exclusive => 1,
                FullscreenMode::Desktop => 0x1001,
                FullscreenMode::Windowed => 0,
            };
            self.sdl.checked(
                (self.sdl.sdl_set_window_fullscreen)(window, mode),
                "SDL_SetWindowFullscreen restore",
            )?;
            if state.maximized {
                (self.sdl.sdl_maximize_window)(window);
            }
            if state.minimized {
                (self.sdl.sdl_minimize_window)(window);
            }
        }
        self.set_visible(state.visible)?;
        if state.visible && state.focused && !state.minimized {
            // SAFETY: the window handle is live.
            unsafe {
                (self.sdl.sdl_raise_window)(window);
            }
        }
        Ok(())
    }

    /// Resize the window.
    pub fn set_size(&mut self, width: i32, height: i32) -> Result<()> {
        for dimension in [width, height] {
            if dimension <= 0 || dimension > 16384 {
                return Err(Error::OutOfRange(
                    "SDL window dimensions must be integers in 1..16384".to_string(),
                ));
            }
        }
        if self.render_reserved() {
            return Err(Error::InvalidInput(
                "SDL render context is reserved for the worker".to_string(),
            ));
        }
        let window = self.opened()?.window;
        // SAFETY: the window handle is live.
        unsafe {
            (self.sdl.sdl_set_window_size)(window, width, height);
        }
        Ok(())
    }

    /// Whether exclusive fullscreen is active.
    pub fn fullscreen(&self) -> Result<bool> {
        Ok(self.flags()? & 1 != 0)
    }

    /// Enter or leave desktop fullscreen.
    pub fn set_fullscreen(&mut self, enabled: bool) -> Result<()> {
        if self.render_reserved() {
            return Err(Error::InvalidInput(
                "SDL render context is reserved for the worker".to_string(),
            ));
        }
        let window = self.opened()?.window;
        // SAFETY: the window handle is live.
        unsafe {
            self.sdl.checked(
                (self.sdl.sdl_set_window_fullscreen)(window, if enabled { 0x1001 } else { 0 }),
                "SDL_SetWindowFullscreen",
            )?;
        }
        Ok(())
    }

    /// One selected gamma window, on one display. See [`SdlGammaCapability`].
    pub fn begin_gamma(&mut self) -> Result<SdlGammaLease> {
        let window = self.opened()?.window;
        if gamma_lease_owner().lock().expect("gamma owner").is_some() {
            return Err(Error::InvalidInput(
                "SDL display gamma already has an owner".to_string(),
            ));
        }
        let display = self.display()?;
        // SAFETY: the window handle is live; ramp buffers are live for each call.
        let (capability, original) = unsafe {
            if display.count != 1 {
                (
                    SdlGammaCapability::Unavailable {
                        reason: "gamma requires the single-display ownership profile".to_string(),
                    },
                    None,
                )
            } else if self.flags()? & 0x200 == 0 {
                (
                    SdlGammaCapability::Unavailable {
                        reason: "gamma acquisition requires actual native input focus".to_string(),
                    },
                    None,
                )
            } else {
                let mut red = [0u16; 256];
                let mut green = [0u16; 256];
                let mut blue = [0u16; 256];
                if (self.sdl.sdl_get_window_gamma_ramp)(window, red.as_mut_ptr(), green.as_mut_ptr(), blue.as_mut_ptr())
                    < 0
                {
                    (
                        SdlGammaCapability::Unsupported {
                            reason: format!("SDL_GetWindowGammaRamp: {}", self.sdl.error()),
                        },
                        None,
                    )
                } else if (self.sdl.sdl_set_window_gamma_ramp)(window, red.as_ptr(), green.as_ptr(), blue.as_ptr()) < 0
                {
                    (
                        SdlGammaCapability::Unsupported {
                            reason: format!("SDL_SetWindowGammaRamp: {}", self.sdl.error()),
                        },
                        None,
                    )
                } else {
                    (
                        SdlGammaCapability::ApiAccepted {
                            display_index: display.index,
                            display_name: display.name.clone(),
                        },
                        Some(GammaRamp { red, green, blue }),
                    )
                }
            }
        };
        let id = next_lease_id();
        *gamma_lease_owner().lock().expect("gamma owner") = Some(id);
        let lease = SdlGammaLease {
            sdl: Arc::clone(&self.sdl),
            id,
            window,
            display_index: display.index,
            display_name: display.name,
            capability: Arc::new(Mutex::new(capability)),
            original: Arc::new(Mutex::new(original)),
            closed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            owner: self.owner,
        };
        self.gamma = Some(lease.clone());
        Ok(lease)
    }

    /// SDL text and relative mouse modes are process-global; one gameplay owner leases both.
    pub fn begin_input(&mut self) -> Result<SdlInputLease> {
        self.opened()?;
        if input_lease_owner().lock().expect("input owner").is_some() {
            return Err(Error::InvalidInput(
                "SDL gameplay input already has an owner".to_string(),
            ));
        }
        // SAFETY: no arguments.
        unsafe {
            (self.sdl.sdl_start_text_input)();
        }
        let id = next_lease_id();
        *input_lease_owner().lock().expect("input owner") = Some(id);
        let lease = SdlInputLease {
            sdl: Arc::clone(&self.sdl),
            id,
            relative: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            entry: Arc::clone(&self.entry),
        };
        self.input = Some(lease.clone());
        Ok(lease)
    }

    /// Pump SDL events and drain this window's routed queue.
    pub fn poll_events(&mut self) -> Result<Vec<SdlEvent>> {
        self.opened()?;
        if let Some(gamma) = &self.gamma {
            gamma.synchronize();
        }
        let mut bytes = [0u8; 56];
        // SAFETY: no arguments.
        unsafe {
            (self.sdl.sdl_pump_events)();
        }
        // Controller events have a separate device owner. Window polling must not consume them.
        let ranges: [(u32, u32); 2] = [(0, 0x64f), (0x670, 0xffff)];
        for (minimum, maximum) in ranges {
            loop {
                // SAFETY: the event buffer is live for the call.
                let count = unsafe { (self.sdl.sdl_peep_events)(bytes.as_mut_ptr(), 1, 2, minimum, maximum) };
                // SAFETY: result is SDL's return for this operation.
                unsafe {
                    self.sdl.checked(count, "SDL_PeepEvents window")?;
                }
                if count == 0 {
                    break;
                }
                let event = decode_sdl_event(&bytes)?;
                if SdlJoystick::queue_event(&event) {
                    continue;
                }
                let event_type = read_u32(&bytes, 0);
                let targeted = event_type == 0x200
                    || (0x300..=0x305).contains(&event_type)
                    || (0x400..=0x403).contains(&event_type);
                let map = windows().lock().expect("window map");
                if targeted {
                    let id = read_u32(&bytes, 8);
                    if let Some(entry) = map.get(&id) {
                        entry.pending.lock().expect("pending").push(event);
                    }
                } else {
                    for entry in map.values() {
                        entry.pending.lock().expect("pending").push(event.clone());
                    }
                }
            }
        }
        let mut pending = self.entry.pending.lock().expect("pending");
        let events = std::mem::take(&mut *pending);
        drop(pending);
        if let Some(gamma) = &self.gamma {
            gamma.synchronize();
        }
        Ok(events)
    }

    /// Present an RGBA frame on a CPU window.
    pub fn present(&mut self, rgba: &[u8]) -> Result<()> {
        let (width, height) = self.drawable_size_signed()?;
        if rgba.len() != width as usize * height as usize * 4 {
            return Err(Error::InvalidInput(
                "RGBA framebuffer size does not match the window".to_string(),
            ));
        }
        let (renderer, mut texture) = {
            let resources = self.opened()?;
            match resources.kind {
                ResourceKind::Cpu { renderer, texture, .. } => (renderer, texture),
                ResourceKind::Gl { .. } => {
                    return Err(Error::InvalidInput("present requires a CPU window".to_string()));
                }
            }
        };
        let sdl = Arc::clone(&self.sdl);
        // SAFETY: handles are live; the framebuffer is live for the calls.
        unsafe {
            let (tex_width, tex_height) = {
                let resources = self.opened()?;
                match resources.kind {
                    ResourceKind::Cpu { width, height, .. } => (width, height),
                    ResourceKind::Gl { .. } => {
                        return Err(Error::InvalidInput("present requires a CPU window".to_string()));
                    }
                }
            };
            if tex_width != width || tex_height != height {
                let replacement = (sdl.sdl_create_texture)(renderer, RGBA32, 1, width, height);
                if replacement.is_null() {
                    return Err(Error::native("SDL_CreateTexture resize", sdl.error()));
                }
                if let Err(error) = sdl.checked(
                    (sdl.sdl_set_texture_blend_mode)(replacement, 0),
                    "SDL_SetTextureBlendMode resize",
                ) {
                    (sdl.sdl_destroy_texture)(replacement);
                    return Err(error);
                }
                (sdl.sdl_destroy_texture)(texture);
                texture = replacement;
                if let Some(resources) = self.resources.as_mut() {
                    if let ResourceKind::Cpu {
                        texture: slot,
                        width: slot_width,
                        height: slot_height,
                        ..
                    } = &mut resources.kind
                    {
                        *slot = texture;
                        *slot_width = width;
                        *slot_height = height;
                    }
                }
                self.has_frame = false;
            }
            sdl.checked(
                (sdl.sdl_update_texture)(texture, std::ptr::null(), rgba.as_ptr(), width * 4),
                "SDL_UpdateTexture",
            )?;
            sdl.checked(
                (sdl.sdl_render_copy)(renderer, texture, std::ptr::null(), std::ptr::null()),
                "SDL_RenderCopy",
            )?;
            (sdl.sdl_render_present)(renderer);
        }
        self.has_frame = true;
        Ok(())
    }

    /// Read back the presented CPU frame.
    pub fn read_pixels(&mut self) -> Result<Vec<u8>> {
        let size = self.drawable_size_signed()?;
        let resources = self.opened()?;
        let (renderer, texture) = match resources.kind {
            ResourceKind::Cpu { renderer, texture, .. } => (renderer, texture),
            ResourceKind::Gl { .. } => {
                return Err(Error::InvalidInput("readPixels requires a CPU window".to_string()));
            }
        };
        if !self.has_frame {
            return Err(Error::InvalidInput("no framebuffer has been presented".to_string()));
        }
        let mut rgba = vec![0u8; size.0 as usize * size.1 as usize * 4];
        // SAFETY: handles are live; SDL invalidates the backbuffer on present,
        // so the retained texture is replayed first.
        unsafe {
            self.sdl.checked(
                (self.sdl.sdl_render_copy)(renderer, texture, std::ptr::null(), std::ptr::null()),
                "SDL_RenderCopy readback",
            )?;
            self.sdl.checked(
                (self.sdl.sdl_render_read_pixels)(renderer, std::ptr::null(), RGBA32, rgba.as_mut_ptr(), size.0 * 4),
                "SDL_RenderReadPixels",
            )?;
        }
        Ok(rgba)
    }

    /// Make the GL context current on this thread.
    pub fn make_current(&mut self) -> Result<()> {
        let resources = self.opened()?;
        let (context, window) = match resources.kind {
            ResourceKind::Gl { context, .. } => (context, resources.window),
            ResourceKind::Cpu { .. } => {
                return Err(Error::InvalidInput("makeCurrent requires a GL window".to_string()));
            }
        };
        if self.render_lease.is_some() {
            return Err(Error::InvalidInput(
                "SDL render context is reserved for the worker".to_string(),
            ));
        }
        // SAFETY: the window and context handles are live.
        unsafe {
            let current = if self.render_enabled {
                context
            } else {
                std::ptr::null_mut()
            };
            self.sdl
                .checked((self.sdl.sdl_gl_make_current)(window, current), "SDL_GL_MakeCurrent")?;
        }
        Ok(())
    }

    /// Whether rendering calls reach the context.
    #[must_use]
    pub fn rendering_enabled(&self) -> bool {
        self.render_enabled
    }

    /// Attach or detach the GL context for diagnostics.
    pub fn set_rendering_enabled(&mut self, enabled: bool) -> Result<()> {
        let resources = self.opened()?;
        let (context, window) = match resources.kind {
            ResourceKind::Gl { context, .. } => (context, resources.window),
            ResourceKind::Cpu { .. } => {
                return Err(Error::InvalidInput(
                    "setRenderingEnabled requires a GL window".to_string(),
                ));
            }
        };
        if self.render_lease.is_some() {
            return Err(Error::InvalidInput(
                "SDL render context is reserved for the worker".to_string(),
            ));
        }
        // SAFETY: the window and context handles are live.
        unsafe {
            let current = if enabled { context } else { std::ptr::null_mut() };
            self.sdl.checked(
                (self.sdl.sdl_gl_make_current)(window, current),
                "SDL_GL_MakeCurrent diagnostic",
            )?;
        }
        self.render_enabled = enabled;
        Ok(())
    }

    /// Detach the GL context into a worker transfer token.
    pub fn detach_render_context(&mut self) -> Result<SdlRenderContextTransfer> {
        let resources = self.opened()?;
        let (context, window) = match resources.kind {
            ResourceKind::Gl { context, .. } => (context, resources.window),
            ResourceKind::Cpu { .. } => {
                return Err(Error::InvalidInput(
                    "detachRenderContext requires a GL window".to_string(),
                ));
            }
        };
        if self.render_lease.is_some() {
            return Err(Error::InvalidInput(
                "SDL render context is reserved for the worker".to_string(),
            ));
        }
        if self.procedure_leases.load(Ordering::SeqCst) != 0 {
            return Err(Error::InvalidInput(
                "close GL procedure tables before transferring the context".to_string(),
            ));
        }
        // SAFETY: the window and context handles are live.
        unsafe {
            self.sdl.checked(
                (self.sdl.sdl_gl_make_current)(window, context),
                "SDL_GL_MakeCurrent transfer",
            )?;
            if self.render_sdl.is_none() {
                self.render_sdl = Some(RenderSdl::load(&self.lib_options)?);
            }
            let render_sdl = self.render_sdl.clone().expect("just loaded");
            let lease = SdlRenderContextLease::detach(window, context, self.id, render_sdl, self.owner)?;
            *self.entry.transfer_state.lock().expect("transfer state") = Some(lease.state());
            let transfer = lease.transfer();
            self.render_lease = Some(lease);
            Ok(transfer)
        }
    }

    /// Restore a detached or parked worker context.
    pub fn restore_render_context(&mut self) -> Result<()> {
        self.opened()?;
        if let Some(lease) = &self.render_lease {
            lease.restore()?;
            self.render_lease = None;
            *self.entry.transfer_state.lock().expect("transfer state") = None;
        }
        if !self.render_enabled {
            self.make_current()?;
        }
        Ok(())
    }

    /// Resolve one GL entry point through the current context.
    pub fn get_gl_proc_address(&mut self, name: &str) -> Result<*mut c_void> {
        if name.is_empty() {
            return Err(Error::InvalidInput("GL procedure name is empty".to_string()));
        }
        let symbol = c_string(name)?;
        self.make_current()?;
        // SAFETY: the name buffer is live for the call.
        let address = unsafe { (self.sdl.sdl_gl_get_proc_address)(symbol.as_ptr()) };
        if address.is_null() {
            // SAFETY: error read immediately after the failing call.
            return Err(Error::native(format!("SDL_GL_GetProcAddress {name}"), unsafe {
                self.sdl.error()
            }));
        }
        Ok(address)
    }

    /// Hold the context alive for a GL procedure table.
    pub fn retain_procedures(&mut self) -> Result<ProcedureGuard> {
        self.make_current()?;
        Ok(ProcedureGuard::new(Arc::clone(&self.procedure_leases)))
    }

    /// Swap front/back buffers on a GL window.
    pub fn swap(&mut self) -> Result<()> {
        let resources = self.opened()?;
        let (context, window) = match resources.kind {
            ResourceKind::Gl { context, .. } => (context, resources.window),
            ResourceKind::Cpu { .. } => {
                return Err(Error::InvalidInput("swap requires a GL window".to_string()));
            }
        };
        self.make_current()?;
        // SAFETY: the window and context handles are live.
        unsafe {
            // SDL requires a current window to swap; Mac flushBuffer did not.
            if !self.render_enabled {
                self.sdl.checked(
                    (self.sdl.sdl_gl_make_current)(window, context),
                    "SDL_GL_MakeCurrent swap",
                )?;
            }
            (self.sdl.sdl_gl_swap_window)(window);
            if !self.render_enabled {
                self.sdl.checked(
                    (self.sdl.sdl_gl_make_current)(window, std::ptr::null_mut()),
                    "SDL_GL_MakeCurrent restore diagnostic",
                )?;
            }
        }
        Ok(())
    }

    /// Current swap interval.
    pub fn swap_interval(&mut self) -> Result<i32> {
        self.make_current()?;
        // SAFETY: no arguments.
        Ok(unsafe { (self.sdl.sdl_gl_get_swap_interval)() })
    }

    /// Set the swap interval (-1, 0, or 1).
    pub fn set_swap_interval(&mut self, interval: i32) -> Result<()> {
        if interval != -1 && interval != 0 && interval != 1 {
            return Err(Error::OutOfRange("SDL swap interval must be -1, 0, or 1".to_string()));
        }
        self.make_current()?;
        // SAFETY: the interval was validated.
        unsafe {
            self.sdl
                .checked((self.sdl.sdl_gl_set_swap_interval)(interval), "SDL_GL_SetSwapInterval")?;
        }
        Ok(())
    }

    /// Inject a synthetic event into the SDL queue.
    pub fn push_event(&mut self, event: &SdlInjectedEvent) -> Result<()> {
        self.opened()?;
        let bytes = encode_sdl_event(event, self.id)?;
        // SAFETY: the event record is live for the call.
        let result = unsafe { (self.sdl.sdl_push_event)(bytes.as_ptr()) };
        // SAFETY: result is SDL's return for this operation.
        unsafe {
            self.sdl.checked(result, "SDL_PushEvent")?;
        }
        if result == 0 {
            return Err(Error::native("SDL_PushEvent", "SDL event was filtered".to_string()));
        }
        Ok(())
    }

    /// Close the window, releasing every native resource. Idempotent once closed.
    pub fn close(&mut self) -> Result<()> {
        if self.resources.is_none() {
            return Ok(());
        }
        if self.procedure_leases.load(Ordering::SeqCst) != 0 {
            return Err(Error::InvalidInput(
                "close GL procedure tables before closing the window".to_string(),
            ));
        }
        self.restore_render_context()?;
        Self::require_window_list_ownership()?;
        let mut errors = Vec::new();
        if let Some(gamma) = self.gamma.take() {
            if let Err(error) = gamma.close() {
                errors.push(error);
            }
        }
        if let Some(input) = self.input.take() {
            if let Err(error) = input.close() {
                errors.push(error);
            }
        }
        let resources = self.resources.take().expect("checked");
        windows().lock().expect("window map").remove(&self.id);
        // SAFETY: every handle below is live until its destroy call.
        unsafe {
            match resources.kind {
                ResourceKind::Cpu { renderer, texture, .. } => {
                    (self.sdl.sdl_destroy_texture)(texture);
                    (self.sdl.sdl_destroy_renderer)(renderer);
                }
                ResourceKind::Gl { context, driver } => {
                    (self.sdl.sdl_gl_delete_context)(context);
                    if driver.is_some() {
                        (self.sdl.sdl_gl_unload_library)();
                    }
                }
            }
            (self.sdl.sdl_destroy_window)(resources.window);
            (self.sdl.sdl_quit_sub_system)(VIDEO_SUBSYSTEM);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(Error::aggregate("SDL window cleanup failed", errors))
        }
    }

    /// Input lease held by this window, if any.
    #[must_use]
    pub fn input_lease(&self) -> Option<&SdlInputLease> {
        self.input.as_ref()
    }

    /// Gamma lease held by this window, if any.
    #[must_use]
    pub fn gamma_lease(&self) -> Option<&SdlGammaLease> {
        self.gamma.as_ref()
    }
}

impl Drop for SdlWindow {
    fn drop(&mut self) {
        self.close().ok();
    }
}

impl SdlRenderContext for SdlWindow {
    fn drawable_size(&mut self) -> Result<(u32, u32)> {
        let (width, height) = self.drawable_size_signed()?;
        Ok((width as u32, height as u32))
    }

    fn rendering_enabled(&self) -> bool {
        self.render_enabled
    }

    fn set_rendering_enabled(&mut self, enabled: bool) -> Result<()> {
        self.set_rendering_enabled(enabled)
    }

    fn make_current(&mut self) -> Result<()> {
        self.make_current()
    }

    fn get_gl_proc_address(&mut self, name: &str) -> Result<*mut c_void> {
        self.get_gl_proc_address(name)
    }

    fn retain_procedures(&mut self) -> Result<ProcedureGuard> {
        self.retain_procedures()
    }

    fn swap(&mut self) -> Result<()> {
        self.swap()
    }
}

/// Backend kind of an open window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SdlBackendKind {
    /// Software renderer.
    Cpu,
    /// OpenGL context.
    Gl,
}

/// Gameplay input lease: process-global text input and relative mouse mode.
#[derive(Clone)]
pub struct SdlInputLease {
    sdl: Arc<Sdl2>,
    id: u64,
    relative: Arc<std::sync::atomic::AtomicBool>,
    entry: Arc<WindowEntry>,
}

impl SdlInputLease {
    /// Whether the lease was closed or superseded.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        *input_lease_owner().lock().expect("input owner") != Some(self.id)
    }

    /// Enable or disable relative mouse mode.
    pub fn set_relative_mouse(&self, enabled: bool) -> Result<()> {
        if self.is_closed() {
            return Err(Error::Closed("SDL input lease".to_string()));
        }
        let relative = self.relative.load(Ordering::SeqCst);
        // SAFETY: no arguments.
        let current = unsafe { (self.sdl.sdl_get_relative_mouse_mode)() != 0 };
        if relative == enabled && current == enabled {
            return Ok(());
        }
        // SAFETY: the mode flag is validated by construction.
        unsafe {
            self.sdl.checked(
                (self.sdl.sdl_set_relative_mouse_mode)(i32::from(enabled)),
                "SDL_SetRelativeMouseMode",
            )?;
        }
        self.relative.store(enabled, Ordering::SeqCst);
        Ok(())
    }

    /// Release relative mode and text input. Idempotent.
    pub fn close(&self) -> Result<()> {
        if self.is_closed() {
            return Ok(());
        }
        let mut errors = Vec::new();
        if self.relative.load(Ordering::SeqCst) {
            // SAFETY: the mode flag is validated by construction.
            if let Err(error) = unsafe {
                self.sdl.checked(
                    (self.sdl.sdl_set_relative_mouse_mode)(0),
                    "SDL_SetRelativeMouseMode release",
                )
            } {
                errors.push(error);
            }
            self.relative.store(false, Ordering::SeqCst);
        }
        // SAFETY: no arguments; SDL_StopTextInput cannot fail, but a panic
        // guard keeps the owner cleared below.
        unsafe {
            (self.sdl.sdl_stop_text_input)();
        }
        *input_lease_owner().lock().expect("input owner") = None;
        let _ = &self.entry;
        if errors.is_empty() {
            Ok(())
        } else {
            Err(Error::aggregate("SDL input release failed", errors))
        }
    }
}

/// Display gamma lease for one window on one display.
#[derive(Clone)]
pub struct SdlGammaLease {
    sdl: Arc<Sdl2>,
    id: u64,
    window: *mut c_void,
    display_index: i32,
    display_name: String,
    capability: Arc<Mutex<SdlGammaCapability>>,
    original: Arc<Mutex<Option<GammaRamp>>>,
    closed: Arc<std::sync::atomic::AtomicBool>,
    owner: thread::ThreadId,
}

impl SdlGammaLease {
    /// Whether the lease is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst) || *gamma_lease_owner().lock().expect("gamma owner") != Some(self.id)
    }

    /// Current capability, synchronizing topology first.
    #[must_use]
    pub fn capability(&self) -> SdlGammaCapability {
        self.synchronize();
        self.capability.lock().expect("capability").clone()
    }

    /// Retire the lease when the display topology changed.
    pub fn synchronize(&self) {
        if self.is_closed() {
            return;
        }
        let current = self.capability.lock().expect("capability").clone();
        if !matches!(current, SdlGammaCapability::ApiAccepted { .. }) {
            return;
        }
        // SAFETY: the window handle is live while the lease is open.
        let display = unsafe { query_display(&self.sdl, self.window) };
        match display {
            Ok(display)
                if display.count == 1 && display.index == self.display_index && display.name == self.display_name => {}
            _ => {
                *self.capability.lock().expect("capability") = SdlGammaCapability::Retired {
                    reason: "SDL display topology changed; hardware continuity and cross-display restoration are unsupported"
                        .to_string(),
                };
                self.restore().ok();
            }
        }
    }

    /// Apply a gamma value in 0.5..=3.0.
    pub fn apply(&self, gamma: f32) -> Result<()> {
        if thread::current().id() != self.owner {
            return Err(Error::InvalidInput(
                "SDL window lifetime belongs to the owning thread".to_string(),
            ));
        }
        if self.is_closed() {
            return Err(Error::Closed("SDL gamma lease".to_string()));
        }
        if !gamma.is_finite() || gamma < 0.5 || gamma > 3.0 {
            return Err(Error::OutOfRange("gamma requires a finite value in 0.5..3".to_string()));
        }
        self.synchronize();
        let capability = self.capability.lock().expect("capability").clone();
        if !matches!(capability, SdlGammaCapability::ApiAccepted { .. }) {
            return Err(Error::InvalidInput(format!(
                "SDL gamma is retired or unavailable: {capability:?}"
            )));
        }
        let mut ramp = [0u16; 256];
        // SAFETY: the ramp buffer is live for the calls.
        unsafe {
            // Linux GLimp_SetGamma consumes r_gamma itself, not the renderer's overbright table.
            (self.sdl.sdl_calculate_gamma_ramp)(gamma, ramp.as_mut_ptr());
            self.sdl.checked(
                (self.sdl.sdl_set_window_gamma_ramp)(self.window, ramp.as_ptr(), ramp.as_ptr(), ramp.as_ptr()),
                "SDL_SetWindowGammaRamp",
            )?;
        }
        Ok(())
    }

    fn restore(&self) -> Result<()> {
        let mut original = self.original.lock().expect("original");
        let Some(ramp) = original.take() else {
            return Ok(());
        };
        // SAFETY: the window handle is live while the lease is open.
        let same = unsafe { query_display(&self.sdl, self.window) }
            .is_ok_and(|display| display.index == self.display_index && display.name == self.display_name);
        if !same {
            return Ok(());
        }
        // SAFETY: ramp buffers are live for the call.
        unsafe {
            self.sdl.checked(
                (self.sdl.sdl_set_window_gamma_ramp)(
                    self.window,
                    ramp.red.as_ptr(),
                    ramp.green.as_ptr(),
                    ramp.blue.as_ptr(),
                ),
                "SDL_SetWindowGammaRamp restore",
            )?;
        }
        Ok(())
    }

    /// Restore the original ramp and release the lease. Idempotent.
    pub fn close(&self) -> Result<()> {
        if self.is_closed() {
            return Ok(());
        }
        let result = self.restore();
        self.closed.store(true, Ordering::SeqCst);
        *gamma_lease_owner().lock().expect("gamma owner") = None;
        result
    }
}

/// Joystick input profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoystickProfile {
    /// Unix axis/button profile (`/dev/js0..3` replacement).
    Linux,
    /// Windows absolute U/V polling profile.
    Windows,
}

/// Owned joystick event record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SdlJoystickEvent {
    /// Axis motion.
    Axis {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Device instance.
        instance: i32,
        /// Axis index.
        axis: u8,
        /// Axis value.
        value: i16,
    },
    /// Hat motion.
    Hat {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Device instance.
        instance: i32,
        /// Hat index.
        hat: u8,
        /// Hat value.
        value: u8,
    },
    /// Button transition.
    Button {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Device instance.
        instance: i32,
        /// Button index.
        button: u8,
        /// True for press, false for release.
        down: bool,
    },
    /// Device removed.
    Removed {
        /// Event timestamp in ms.
        timestamp: u32,
        /// Device instance.
        instance: i32,
    },
}

impl SdlJoystickEvent {
    fn instance(&self) -> i32 {
        match self {
            Self::Axis { instance, .. }
            | Self::Hat { instance, .. }
            | Self::Button { instance, .. }
            | Self::Removed { instance, .. } => *instance,
        }
    }

    fn from_event(event: &SdlEvent) -> Option<Self> {
        match event {
            SdlEvent::JoystickAxis {
                timestamp,
                instance,
                axis,
                value,
            } => Some(Self::Axis {
                timestamp: *timestamp,
                instance: *instance,
                axis: *axis,
                value: *value,
            }),
            SdlEvent::JoystickHat {
                timestamp,
                instance,
                hat,
                value,
            } => Some(Self::Hat {
                timestamp: *timestamp,
                instance: *instance,
                hat: *hat,
                value: *value,
            }),
            SdlEvent::JoystickButton {
                timestamp,
                instance,
                button,
                down,
            } => Some(Self::Button {
                timestamp: *timestamp,
                instance: *instance,
                button: *button,
                down: *down,
            }),
            SdlEvent::JoystickRemoved { timestamp, instance } => Some(Self::Removed {
                timestamp: *timestamp,
                instance: *instance,
            }),
            _ => None,
        }
    }
}

struct JoystickSink {
    pending: Arc<Mutex<Vec<SdlJoystickEvent>>>,
}

fn joysticks() -> &'static Mutex<HashMap<i32, JoystickSink>> {
    static OPENED: OnceLock<Mutex<HashMap<i32, JoystickSink>>> = OnceLock::new();
    OPENED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// SDL replaces `/dev/js0..3` selection. Events retain every button edge for `IN_JoyMove`.
pub struct SdlJoystick {
    sdl: Arc<Sdl2>,
    pointer: *mut c_void,
    instance: i32,
    name: String,
    axes: i32,
    buttons: i32,
    hats: i32,
    pending: Arc<Mutex<Vec<SdlJoystickEvent>>>,
    owner: thread::ThreadId,
    _not_send: PhantomData<*const ()>,
}

impl SdlJoystick {
    /// Route a decoded event to its device owner. Returns true for joystick
    /// events (consumed from window streams even without an owner).
    pub(crate) fn queue_event(event: &SdlEvent) -> bool {
        let Some(record) = SdlJoystickEvent::from_event(event) else {
            return false;
        };
        let map = joysticks().lock().expect("joystick map");
        if let Some(sink) = map.get(&record.instance()) {
            sink.pending.lock().expect("pending").push(record);
        }
        true
    }

    /// Open the first of up to four joysticks, warning about unmapped controls.
    pub fn open_first(print: impl Fn(&str), profile: JoystickProfile) -> Result<Option<Self>> {
        Self::open_first_with(print, profile, &NativeLibraryOptions::default())
    }

    /// Open with explicit library discovery (tests inject overrides).
    pub fn open_first_with(
        print: impl Fn(&str),
        profile: JoystickProfile,
        lib_options: &NativeLibraryOptions,
    ) -> Result<Option<Self>> {
        // SAFETY: loading maps the image; SDL calls below use validated arguments.
        let sdl = unsafe { Sdl2::load(lib_options)? };
        unsafe {
            Self::initialize(&sdl)?;
            let count = (sdl.sdl_num_joysticks)();
            if let Err(error) = sdl.checked(count, "SDL_NumJoysticks") {
                (sdl.sdl_quit_sub_system)(JOYSTICK_SUBSYSTEM);
                return Err(error);
            }
            let mut pointer: *mut c_void = std::ptr::null_mut();
            for index in 0..count.min(4) {
                pointer = (sdl.sdl_joystick_open)(index);
                if !pointer.is_null() {
                    break;
                }
                print(&format!("SDL_JoystickOpen {index}: {}\n", sdl.error()));
            }
            if pointer.is_null() {
                (sdl.sdl_quit_sub_system)(JOYSTICK_SUBSYSTEM);
                return Ok(None);
            }
            let outcome = Self::describe(&sdl, pointer, &print, profile);
            match outcome {
                Ok((instance, name, axes, buttons, hats)) => {
                    let pending = Arc::new(Mutex::new(Vec::new()));
                    joysticks().lock().expect("joystick map").insert(
                        instance,
                        JoystickSink {
                            pending: Arc::clone(&pending),
                        },
                    );
                    Ok(Some(Self {
                        sdl,
                        pointer,
                        instance,
                        name,
                        axes,
                        buttons,
                        hats,
                        pending,
                        owner: thread::current().id(),
                        _not_send: PhantomData,
                    }))
                }
                Err(error) => {
                    (sdl.sdl_joystick_close)(pointer);
                    (sdl.sdl_quit_sub_system)(JOYSTICK_SUBSYSTEM);
                    Err(error)
                }
            }
        }
    }

    /// # Safety
    ///
    /// The SDL handle must be fully loaded.
    unsafe fn initialize(sdl: &Sdl2) -> Result<()> {
        // SAFETY: hint buffers are live for the call.
        unsafe {
            let key = c_string("SDL_NO_SIGNAL_HANDLERS")?;
            let value = c_string("1")?;
            if (sdl.sdl_set_hint)(key.as_ptr(), value.as_ptr()) != 1 {
                return Err(Error::InvalidInput(
                    "SDL must leave signal handling to the Unix signal owner".to_string(),
                ));
            }
            sdl.checked(
                (sdl.sdl_init_sub_system)(JOYSTICK_SUBSYSTEM),
                "SDL_InitSubSystem joystick",
            )?;
        }
        Ok(())
    }

    /// # Safety
    ///
    /// `pointer` must be a live joystick handle.
    unsafe fn describe(
        sdl: &Sdl2,
        pointer: *mut c_void,
        print: &impl Fn(&str),
        profile: JoystickProfile,
    ) -> Result<(i32, String, i32, i32, i32)> {
        // SAFETY: the joystick handle is live.
        unsafe {
            let instance = (sdl.sdl_joystick_instance_id)(pointer);
            let axes = (sdl.sdl_joystick_num_axes)(pointer);
            let buttons = (sdl.sdl_joystick_num_buttons)(pointer);
            let hats = (sdl.sdl_joystick_num_hats)(pointer);
            let balls = (sdl.sdl_joystick_num_balls)(pointer);
            for value in [instance, axes, buttons, hats, balls] {
                sdl.checked(value, "SDL joystick description")?;
            }
            if profile == JoystickProfile::Linux && (hats != 0 || balls != 0) {
                print(&format!(
                    "SDL joystick has {hats} hats and {balls} balls; this Unix axis/button profile does not map them.\n"
                ));
            }
            if profile == JoystickProfile::Windows && balls != 0 {
                print(&format!(
                    "SDL joystick has {balls} relative balls; Windows source mouse motion uses absolute U/V axes.\n"
                ));
            }
            Ok((
                instance,
                c_string_lossy((sdl.sdl_joystick_name)(pointer)),
                axes,
                buttons,
                hats,
            ))
        }
    }

    /// Device instance id.
    #[must_use]
    pub fn instance(&self) -> i32 {
        self.instance
    }

    /// Device name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Axis count.
    #[must_use]
    pub fn axes(&self) -> i32 {
        self.axes
    }

    /// Button count.
    #[must_use]
    pub fn buttons(&self) -> i32 {
        self.buttons
    }

    /// Whether the joystick is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.pointer.is_null()
    }

    /// Pump only joystick devices and remove only their event range, including before video init.
    pub fn poll_events(&mut self, profile: JoystickProfile) -> Result<Vec<SdlJoystickEvent>> {
        if thread::current().id() != self.owner {
            return Err(Error::InvalidInput(
                "SDL joystick belongs to the owning thread".to_string(),
            ));
        }
        if self.pointer.is_null() {
            return Err(Error::Closed("SDL joystick".to_string()));
        }
        let mut bytes = [0u8; 56];
        // SAFETY: the joystick handle is live; the event buffer is live per call.
        unsafe {
            (self.sdl.sdl_joystick_update)();
            loop {
                let count = (self.sdl.sdl_peep_events)(bytes.as_mut_ptr(), 1, 2, 0x600, 0x606);
                self.sdl.checked(count, "SDL_PeepEvents joystick")?;
                if count == 0 {
                    break;
                }
                Self::queue_event(&decode_sdl_event(&bytes)?);
            }
        }
        let mut pending = self.pending.lock().expect("pending");
        let events = std::mem::take(&mut *pending);
        drop(pending);
        if profile == JoystickProfile::Windows {
            if let Some(removed) = events
                .iter()
                .find(|event| matches!(event, SdlJoystickEvent::Removed { .. }))
            {
                return Ok(vec![removed.clone()]);
            }
            // joyGetPosEx polls the current absolute values, including unchanged U/V.
            // SDL event-only state can omit the initial centered or held values.
            let mut state = Vec::new();
            // SAFETY: the joystick handle is live.
            unsafe {
                let timestamp = (self.sdl.sdl_get_ticks)();
                for button in 0..self.buttons.min(32) {
                    state.push(SdlJoystickEvent::Button {
                        timestamp,
                        instance: self.instance,
                        button: button as u8,
                        down: (self.sdl.sdl_joystick_get_button)(self.pointer, button) != 0,
                    });
                }
                for axis in 0..self.axes.min(6) {
                    state.push(SdlJoystickEvent::Axis {
                        timestamp,
                        instance: self.instance,
                        axis: axis as u8,
                        value: (self.sdl.sdl_joystick_get_axis)(self.pointer, axis),
                    });
                }
                if self.hats != 0 {
                    state.push(SdlJoystickEvent::Hat {
                        timestamp,
                        instance: self.instance,
                        hat: 0,
                        value: (self.sdl.sdl_joystick_get_hat)(self.pointer, 0),
                    });
                }
            }
            return Ok(state);
        }
        Ok(events)
    }

    /// Close the joystick and quit its subsystem.
    pub fn close(&mut self) {
        if self.pointer.is_null() {
            return;
        }
        let pointer = self.pointer;
        self.pointer = std::ptr::null_mut();
        joysticks().lock().expect("joystick map").remove(&self.instance);
        self.pending.lock().expect("pending").clear();
        // SAFETY: the joystick handle is live until this call.
        unsafe {
            (self.sdl.sdl_joystick_close)(pointer);
            (self.sdl.sdl_quit_sub_system)(JOYSTICK_SUBSYSTEM);
        }
    }
}

impl Drop for SdlJoystick {
    fn drop(&mut self) {
        self.close();
    }
}

/// Open a URL in the sign-in browser.
pub fn open_sdl_url(url: &str) -> Result<()> {
    open_sdl_url_with(url, &NativeLibraryOptions::default())
}

/// Open a URL with explicit library discovery.
pub fn open_sdl_url_with(url: &str, options: &NativeLibraryOptions) -> Result<()> {
    // SAFETY: loading maps the image; the URL buffer is live for the call.
    unsafe {
        let sdl = Sdl2::load(options)?;
        let target = c_string(url)?;
        if (sdl.sdl_open_url)(target.as_ptr()) != 0 {
            return Err(Error::native("SDL_OpenURL", "could not open the sign-in browser"));
        }
        Ok(())
    }
}

/// Replace the platform clipboard text.
pub fn write_sdl_clipboard(text: &str) -> Result<()> {
    write_sdl_clipboard_with(text, &NativeLibraryOptions::default())
}

/// Replace the clipboard with explicit library discovery.
pub fn write_sdl_clipboard_with(text: &str, options: &NativeLibraryOptions) -> Result<()> {
    // SAFETY: loading maps the image; the text buffer is live for the call.
    unsafe {
        let sdl = Sdl2::load(options)?;
        let value = c_string(text)?;
        sdl.checked((sdl.sdl_set_clipboard_text)(value.as_ptr()), "SDL_SetClipboardText")?;
        Ok(())
    }
}

/// Read the clipboard as source bytes, or `None` when SDL has no text.
pub fn read_sdl_clipboard() -> Result<Option<Vec<u8>>> {
    read_sdl_clipboard_with(&NativeLibraryOptions::default())
}

/// Read the clipboard with explicit library discovery.
pub fn read_sdl_clipboard_with(options: &NativeLibraryOptions) -> Result<Option<Vec<u8>>> {
    // SAFETY: loading maps the image; the SDL allocation is copied before release.
    unsafe {
        let sdl = Sdl2::load(options)?;
        let allocation = (sdl.sdl_get_clipboard_text)();
        if allocation.is_null() {
            return Ok(None);
        }
        // SDL owns the NUL-terminated allocation. Copy before releasing it, without decoding.
        let bytes = c_string_bytes(allocation.cast::<u8>());
        (sdl.sdl_free)(allocation);
        Ok(Some(source_clipboard_bytes(&bytes)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn missing_lib() -> NativeLibraryOptions {
        let mut environment = HashMap::new();
        environment.insert(
            "QUAKE_SDL2_LIBRARY".to_string(),
            "/nonexistent-qa-platform/libSDL2.so".to_string(),
        );
        NativeLibraryOptions {
            environment: Some(environment),
            ..NativeLibraryOptions::default()
        }
    }

    #[test]
    fn clipboard_bytes_keep_leading_delimiters() {
        assert_eq!(source_clipboard_bytes(b"hello\0"), b"hello\0");
        assert_eq!(source_clipboard_bytes(b"\n\rhello\0"), b"\n\rhello\0");
        assert_eq!(source_clipboard_bytes(b"ab\ncd\0"), b"ab\0");
        assert_eq!(source_clipboard_bytes(b"ab\x08cd\0"), b"ab\0");
        assert_eq!(source_clipboard_bytes(b"\0"), b"\0");
    }

    #[test]
    fn display_index_falls_back_to_main() {
        assert_eq!(sdl_display_index(1, 3).unwrap(), 1);
        assert_eq!(sdl_display_index(-1, 3).unwrap(), 0);
        assert_eq!(sdl_display_index(9, 3).unwrap(), 0);
        assert!(sdl_display_index(0, 0).is_err());
    }

    #[test]
    fn matching_display_mode_retains_last_exact() {
        let modes = vec![
            SdlDisplayMode {
                width: 800,
                height: 600,
                color_bits: 32,
                refresh_rate: 60,
            },
            SdlDisplayMode {
                width: 800,
                height: 600,
                color_bits: 32,
                refresh_rate: 75,
            },
            SdlDisplayMode {
                width: 800,
                height: 600,
                color_bits: 16,
                refresh_rate: 75,
            },
        ];
        let request = SdlDisplayModeRequest {
            width: 800,
            height: 600,
            color_bits: 32,
            min_display_refresh: 0,
            max_display_refresh: 0,
        };
        assert_eq!(sdl_matching_display_mode(&modes, &request).unwrap(), Some(1));
        let capped = SdlDisplayModeRequest {
            max_display_refresh: 60,
            ..request
        };
        assert_eq!(sdl_matching_display_mode(&modes, &capped).unwrap(), Some(0));
        let none = SdlDisplayModeRequest {
            min_display_refresh: 120,
            ..request
        };
        assert_eq!(sdl_matching_display_mode(&modes, &none).unwrap(), None);
        let bad = SdlDisplayModeRequest {
            min_display_refresh: 75,
            max_display_refresh: 60,
            ..request
        };
        assert!(sdl_matching_display_mode(&modes, &bad).is_err());
    }

    #[test]
    fn visual_attempts_follow_source_sequence() {
        let attempts = visual_attempts(24, 24, 8);
        assert_eq!(attempts.len(), 16);
        assert_eq!(
            attempts[0],
            VisualAttempt {
                component: 8,
                depth: 24,
                stencil: 8
            }
        );
        assert_eq!(attempts[1].stencil, 0);
        assert_eq!(attempts[2].depth, 16);
        assert_eq!(attempts[3].component, 4);
        assert_eq!(attempts[4].depth, 16);
        assert_eq!(attempts[8].component, 4);
        assert_eq!(attempts[12].stencil, 8);
    }

    #[test]
    fn visual_precision_defaults_zero_to_24() {
        assert_eq!(source_visual_precision(0.0).unwrap(), 24);
        assert_eq!(source_visual_precision(16.9).unwrap(), 16);
        assert_eq!(source_visual_precision(-0.0).unwrap(), 24);
    }

    #[test]
    fn display_mode_color_bits_come_from_format() {
        let mut bytes = [0u8; 24];
        bytes[0..4].copy_from_slice(&RGBA32.to_ne_bytes());
        bytes[4..8].copy_from_slice(&1024i32.to_ne_bytes());
        bytes[8..12].copy_from_slice(&768i32.to_ne_bytes());
        bytes[12..16].copy_from_slice(&60i32.to_ne_bytes());
        let mode = display_mode_from_bytes(&bytes);
        assert_eq!(mode.width, 1024);
        assert_eq!(mode.height, 768);
        assert_eq!(mode.refresh_rate, 60);
        assert_eq!(mode.color_bits, ((RGBA32 >> 8) & 255) as i32);
    }

    fn test_presentation(display_mode: (u32, i32, i32, i32)) -> SdlWindowPresentation {
        SdlWindowPresentation {
            size: (960, 600),
            position: (0, 0),
            display_index: 0,
            display_mode,
            fullscreen: FullscreenMode::Windowed,
            visible: true,
            maximized: false,
            minimized: false,
            focused: false,
        }
    }

    #[test]
    fn zeroed_windowed_capture_skips_display_mode_restore() {
        // Windowed captures record the zeroed sentinel when SDL reports no
        // mode (e.g. a window larger than a default Xvfb screen); restore
        // must skip the mode set for those captures only.
        assert!(!presentation_has_display_mode(&test_presentation((0, 0, 0, 0))));
        assert!(presentation_has_display_mode(&test_presentation((
            RGBA32, 640, 480, 60
        ))));
        assert!(presentation_has_display_mode(&test_presentation((0, 640, 480, 0))));
    }

    fn event_bytes() -> [u8; 56] {
        [0u8; 56]
    }

    #[test]
    fn decode_key_text_mouse_wheel_joystick() {
        let mut key = event_bytes();
        key[0..4].copy_from_slice(&0x300u32.to_ne_bytes());
        key[4..8].copy_from_slice(&42u32.to_ne_bytes());
        key[13] = 1;
        key[16..20].copy_from_slice(&7i32.to_ne_bytes());
        key[20..24].copy_from_slice(&8i32.to_ne_bytes());
        key[24..26].copy_from_slice(&3u16.to_ne_bytes());
        assert!(matches!(
            decode_sdl_event(&key).unwrap(),
            SdlEvent::Key {
                timestamp: 42,
                down: true,
                repeat: true,
                scancode: 7,
                keycode: 8,
                modifiers: 3
            }
        ));
        let mut text = event_bytes();
        text[0..4].copy_from_slice(&0x303u32.to_ne_bytes());
        text[12..17].copy_from_slice(b"hi!\0\0");
        assert!(matches!(
            decode_sdl_event(&text).unwrap(),
            SdlEvent::Text { text, .. } if text == "hi!"
        ));
        let mut bad_text = event_bytes();
        bad_text[0..4].copy_from_slice(&0x303u32.to_ne_bytes());
        bad_text[12] = 0xff;
        bad_text[13] = 0;
        assert!(decode_sdl_event(&bad_text).is_err());
        let mut wheel = event_bytes();
        wheel[0..4].copy_from_slice(&0x403u32.to_ne_bytes());
        wheel[16..20].copy_from_slice(&1i32.to_ne_bytes());
        wheel[20..24].copy_from_slice(&(-1i32).to_ne_bytes());
        wheel[24..28].copy_from_slice(&1u32.to_ne_bytes());
        wheel[28..32].copy_from_slice(&1.5f32.to_ne_bytes());
        wheel[32..36].copy_from_slice(&(-2.5f32).to_ne_bytes());
        assert!(matches!(
            decode_sdl_event(&wheel).unwrap(),
            SdlEvent::MouseWheel { flipped: true, .. }
        ));
        let mut axis = event_bytes();
        axis[0..4].copy_from_slice(&0x600u32.to_ne_bytes());
        axis[8..12].copy_from_slice(&9i32.to_ne_bytes());
        axis[12] = 2;
        axis[16..18].copy_from_slice(&(-300i16).to_ne_bytes());
        assert!(matches!(
            decode_sdl_event(&axis).unwrap(),
            SdlEvent::JoystickAxis {
                instance: 9,
                axis: 2,
                value: -300,
                ..
            }
        ));
        let mut unknown = event_bytes();
        unknown[0..4].copy_from_slice(&0x9999u32.to_ne_bytes());
        assert!(matches!(
            decode_sdl_event(&unknown).unwrap(),
            SdlEvent::Unsupported { event_type: 0x9999, .. }
        ));
    }

    #[test]
    fn inject_round_trip() {
        let events = vec![
            SdlInjectedEvent::Quit { timestamp: 1 },
            SdlInjectedEvent::Key {
                timestamp: 2,
                down: true,
                repeat: false,
                scancode: 10,
                keycode: 20,
                modifiers: 4,
            },
            SdlInjectedEvent::MouseMotion {
                timestamp: 3,
                buttons: 1,
                x: 5,
                y: 6,
                dx: 1,
                dy: -1,
            },
            SdlInjectedEvent::MouseButton {
                timestamp: 4,
                down: false,
                button: 3,
                clicks: 2,
                x: 7,
                y: 8,
            },
            SdlInjectedEvent::Window {
                timestamp: 5,
                event: 14,
                data1: 640,
                data2: 480,
            },
        ];
        for event in events {
            let bytes = encode_sdl_event(&event, 99).unwrap();
            assert_eq!(read_u32(&bytes, 8), 99);
            let decoded = decode_sdl_event(&bytes).unwrap();
            match (event, decoded) {
                (SdlInjectedEvent::Quit { timestamp }, SdlEvent::Quit { timestamp: back }) => {
                    assert_eq!(timestamp, back);
                }
                (
                    SdlInjectedEvent::Key {
                        scancode,
                        keycode,
                        modifiers,
                        ..
                    },
                    SdlEvent::Key {
                        scancode: s,
                        keycode: k,
                        modifiers: m,
                        down: true,
                        ..
                    },
                ) => {
                    assert_eq!((scancode, keycode, modifiers), (s, k, m));
                }
                (
                    SdlInjectedEvent::MouseMotion { x, y, dx, dy, .. },
                    SdlEvent::MouseMotion {
                        x: x2,
                        y: y2,
                        dx: dx2,
                        dy: dy2,
                        ..
                    },
                ) => assert_eq!((x, y, dx, dy), (x2, y2, dx2, dy2)),
                (
                    SdlInjectedEvent::MouseButton {
                        button, clicks, down, ..
                    },
                    SdlEvent::MouseButton {
                        button: b,
                        clicks: c,
                        down: d,
                        ..
                    },
                ) => assert_eq!((button, clicks, down), (b, c, d)),
                (
                    SdlInjectedEvent::Window {
                        event, data1, data2, ..
                    },
                    SdlEvent::Window {
                        event: e,
                        data1: d1,
                        data2: d2,
                        ..
                    },
                ) => assert_eq!((event, data1, data2), (e, d1, d2)),
                _ => panic!("inject round trip mismatch"),
            }
        }
    }

    #[test]
    fn missing_sdl_names_library_for_every_entry() {
        let options = missing_lib();
        let window_options = SdlWindowOptions {
            title: "qa".to_string(),
            width: 64,
            height: 64,
            ..SdlWindowOptions::default()
        };
        let Err(error) = SdlWindow::open_with(&window_options, &options) else {
            panic!("expected failure")
        };
        assert!(error.is_unavailable(), "{error}");
        assert!(error.to_string().contains("sdl2"), "{error}");
        let Err(error) = SdlJoystick::open_first_with(|_| {}, JoystickProfile::Linux, &options) else {
            panic!("expected failure")
        };
        assert!(error.is_unavailable(), "{error}");
        assert!(error.to_string().contains("sdl2"), "{error}");
        let Err(error) = open_sdl_url_with("https://example.invalid", &options) else {
            panic!("expected failure")
        };
        assert!(error.is_unavailable(), "{error}");
        let Err(error) = write_sdl_clipboard_with("text", &options) else {
            panic!("expected failure")
        };
        assert!(error.is_unavailable(), "{error}");
        let Err(error) = read_sdl_clipboard_with(&options) else {
            panic!("expected failure")
        };
        assert!(error.is_unavailable(), "{error}");
    }

    #[test]
    fn window_validation_runs_before_loading() {
        let options = missing_lib();
        let bad = SdlWindowOptions {
            title: "qa".to_string(),
            width: 0,
            height: 64,
            ..SdlWindowOptions::default()
        };
        let Err(error) = SdlWindow::open_with(&bad, &options) else {
            panic!("expected failure")
        };
        assert!(!error.is_unavailable(), "{error}");
        let bad_gl = SdlWindowOptions {
            title: "qa".to_string(),
            width: 64,
            height: 64,
            backend: SdlBackend::Gl(SdlGlOptions {
                stencil_bits: 99,
                ..SdlGlOptions::default()
            }),
            ..SdlWindowOptions::default()
        };
        assert!(SdlWindow::open_with(&bad_gl, &options).is_err());
        let bad_driver = SdlWindowOptions {
            title: "qa".to_string(),
            width: 64,
            height: 64,
            backend: SdlBackend::Gl(SdlGlOptions {
                driver: Some("vendor-gl.dll".to_string()),
                ..SdlGlOptions::default()
            }),
            ..SdlWindowOptions::default()
        };
        let Err(error) = SdlWindow::open_with(&bad_driver, &options) else {
            panic!("expected failure")
        };
        assert!(error.to_string().contains("driver"), "{error}");
    }

    #[test]
    fn joystick_routing_consumes_device_events() {
        let axis = SdlEvent::JoystickAxis {
            timestamp: 1,
            instance: 4242,
            axis: 0,
            value: 5,
        };
        assert!(SdlJoystick::queue_event(&axis));
        assert!(!SdlJoystick::queue_event(&SdlEvent::Quit { timestamp: 1 }));
    }

    #[test]
    fn live_open_reports_honestly() {
        // Passes with or without system SDL: a real window is exercised when
        // available, otherwise the honest absence signal is required.
        let options = SdlWindowOptions {
            title: "qa-platform probe".to_string(),
            width: 64,
            height: 64,
            hidden: true,
            ..SdlWindowOptions::default()
        };
        match SdlWindow::open(&options) {
            Ok(mut window) => {
                assert_eq!(window.backend().unwrap(), SdlBackendKind::Cpu);
                assert!(!window.is_closed());
                let events = window.poll_events().unwrap();
                let _ = events.len();
                window.close().unwrap();
                assert!(window.is_closed());
            }
            Err(error) => assert!(!error.to_string().is_empty(), "{error}"),
        }
        if let Ok(Some(mut joystick)) = SdlJoystick::open_first(|_| {}, JoystickProfile::Linux) {
            let _ = joystick.poll_events(JoystickProfile::Linux).unwrap();
            joystick.close();
        }
    }
}
