//! SDL3 ABI definitions follow SDL 3.4 headers, never SDL2 byte offsets.
use crate::clock::Clock;
use qa_core::sys_events::{DeviceId, EventKind, EventTime, SysEvent, SysEventQueue};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr::NonNull;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SdlCommon {
    kind: u32,
    reserved: u32,
    timestamp: u64,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SdlKey {
    common: SdlCommon,
    window: u32,
    which: u32,
    scancode: i32,
    key: u32,
    modifiers: u16,
    raw: u16,
    down: bool,
    repeat: bool,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct SdlText {
    common: SdlCommon,
    window: u32,
    text: *const c_char,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SdlMotion {
    common: SdlCommon,
    window: u32,
    which: u32,
    state: u32,
    x: f32,
    y: f32,
    dx: f32,
    dy: f32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct SdlButton {
    common: SdlCommon,
    window: u32,
    which: u32,
    button: u8,
    down: bool,
    clicks: u8,
    padding: u8,
    x: f32,
    y: f32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct SdlWheel {
    common: SdlCommon,
    window: u32,
    which: u32,
    x: f32,
    y: f32,
    direction: u32,
    mouse_x: f32,
    mouse_y: f32,
    integer_x: i32,
    integer_y: i32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct SdlAxis {
    common: SdlCommon,
    which: u32,
    axis: u8,
    padding: [u8; 3],
    value: i16,
    padding2: u16,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct SdlPadButton {
    common: SdlCommon,
    which: u32,
    button: u8,
    down: bool,
    padding: [u8; 2],
}
#[repr(C)]
#[derive(Clone, Copy)]
struct SdlDevice {
    common: SdlCommon,
    which: u32,
}
#[repr(C)]
union SdlEvent {
    common: SdlCommon,
    key: SdlKey,
    text: SdlText,
    motion: SdlMotion,
    button: SdlButton,
    wheel: SdlWheel,
    axis: SdlAxis,
    pad_button: SdlPadButton,
    device: SdlDevice,
    padding: [u8; 128],
}

const SUBSYSTEMS: u32 = 0x20 | 0x2000; // VIDEO | GAMEPAD
#[link(name = "SDL3")]
unsafe extern "C" {
    fn SDL_InitSubSystem(flags: u32) -> bool;
    fn SDL_QuitSubSystem(flags: u32);
    fn SDL_GetError() -> *const c_char;
    fn SDL_GetCurrentVideoDriver() -> *const c_char;
    fn SDL_CreateWindow(title: *const c_char, w: c_int, h: c_int, flags: u64) -> *mut c_void;
    fn SDL_SyncWindow(window: *mut c_void) -> bool;
    fn SDL_DestroyWindow(window: *mut c_void);
    fn SDL_CreateRenderer(window: *mut c_void, name: *const c_char) -> *mut c_void;
    fn SDL_DestroyRenderer(renderer: *mut c_void);
    fn SDL_SetRenderDrawColor(renderer: *mut c_void, r: u8, g: u8, b: u8, a: u8) -> bool;
    fn SDL_RenderClear(renderer: *mut c_void) -> bool;
    fn SDL_RenderPresent(renderer: *mut c_void) -> bool;
    fn SDL_PollEvent(event: *mut SdlEvent) -> bool;
    fn SDL_StartTextInput(window: *mut c_void) -> bool;
    fn SDL_GetGamepads(count: *mut c_int) -> *mut u32;
    fn SDL_OpenGamepad(id: u32) -> *mut c_void;
    fn SDL_CloseGamepad(gamepad: *mut c_void);
    fn SDL_free(memory: *mut c_void);
    #[cfg(feature = "proof")]
    fn SDL_PushEvent(event: *mut SdlEvent) -> bool;
    #[cfg(feature = "proof")]
    fn SDL_GetKeyFromName(name: *const c_char) -> u32;
    #[cfg(feature = "proof")]
    fn SDL_GetScancodeFromKey(key: u32, modifiers: *mut u16) -> c_int;
}

pub(crate) fn error() -> String {
    unsafe { CStr::from_ptr(SDL_GetError()) }
        .to_string_lossy()
        .into_owned()
}

pub struct Window {
    window: NonNull<c_void>,
    renderer: NonNull<c_void>,
    gamepads: [Option<(u32, NonNull<c_void>)>; 16],
    text: Box<[u8; 8192]>,
    text_cursor: usize,
    text_len: usize,
    text_time: EventTime,
    dropped_text: u64,
    mouse_fraction: [f32; 2],
}
impl Window {
    #[cfg(feature = "proof")]
    pub fn inject_key(&mut self, name: &str, down: bool) -> Result<(), String> {
        let name = std::ffi::CString::new(name).map_err(|_| "invalid key name")?;
        let key = unsafe { SDL_GetKeyFromName(name.as_ptr()) };
        if key == 0 {
            return Err("unknown key name".into());
        }
        let mut event = SdlEvent { padding: [0; 128] };
        event.key = SdlKey {
            common: SdlCommon {
                kind: if down { 0x300 } else { 0x301 },
                ..SdlCommon::default()
            },
            scancode: unsafe { SDL_GetScancodeFromKey(key, std::ptr::null_mut()) },
            key,
            down,
            ..SdlKey::default()
        };
        if !unsafe { SDL_PushEvent(&mut event) } {
            return Err(error());
        }
        Ok(())
    }
    #[cfg(feature = "proof")]
    pub fn inject_mouse(&mut self, dx: i32, dy: i32) -> Result<(), String> {
        let mut event = SdlEvent { padding: [0; 128] };
        event.motion = SdlMotion {
            common: SdlCommon {
                kind: 0x400,
                ..SdlCommon::default()
            },
            dx: dx as f32,
            dy: dy as f32,
            ..SdlMotion::default()
        };
        if !unsafe { SDL_PushEvent(&mut event) } {
            return Err(error());
        }
        Ok(())
    }
    /// Existing development input player only; THE-893 removes it before release.
    /// # Safety
    /// The string must remain alive until SDL polls the submitted text event.
    #[cfg(feature = "proof")]
    pub unsafe fn inject_text(&mut self, text: &CStr) -> Result<(), String> {
        let mut event = SdlEvent { padding: [0; 128] };
        event.text = SdlText {
            common: SdlCommon {
                kind: 0x303,
                ..SdlCommon::default()
            },
            window: 0,
            text: text.as_ptr(),
        };
        if !unsafe { SDL_PushEvent(&mut event) } {
            return Err(error());
        }
        Ok(())
    }
    pub fn video_driver(&self) -> &str {
        unsafe { CStr::from_ptr(SDL_GetCurrentVideoDriver()) }
            .to_str()
            .unwrap_or("unknown")
    }
    pub fn open(width: i32, height: i32) -> Result<Self, String> {
        unsafe {
            if !SDL_InitSubSystem(SUBSYSTEMS) {
                return Err(error());
            }
            let Some(window) = NonNull::new(SDL_CreateWindow(
                c"Quake Anthology Rust".as_ptr(),
                width,
                height,
                0,
            )) else {
                let message = error();
                SDL_QuitSubSystem(SUBSYSTEMS);
                return Err(message);
            };
            // SDL3 window state is asynchronous on Wayland. Complete creation
            // before reporting window_ready or selecting input/capture targets.
            if !SDL_SyncWindow(window.as_ptr()) {
                let message = error();
                SDL_DestroyWindow(window.as_ptr());
                SDL_QuitSubSystem(SUBSYSTEMS);
                return Err(message);
            }
            let Some(renderer) =
                NonNull::new(SDL_CreateRenderer(window.as_ptr(), c"software".as_ptr()))
            else {
                let message = error();
                SDL_DestroyWindow(window.as_ptr());
                SDL_QuitSubSystem(SUBSYSTEMS);
                return Err(message);
            };
            SDL_SetRenderDrawColor(renderer.as_ptr(), 18, 26, 34, 255);
            SDL_StartTextInput(window.as_ptr());
            let mut opened = Self {
                window,
                renderer,
                gamepads: [None; 16],
                text: Box::new([0; 8192]),
                text_cursor: 0,
                text_len: 0,
                text_time: EventTime(0),
                dropped_text: 0,
                mouse_fraction: [0.0; 2],
            };
            let mut count = 0;
            let ids = SDL_GetGamepads(&mut count);
            if !ids.is_null() {
                for &id in std::slice::from_raw_parts(ids, count.max(0) as usize) {
                    opened.open_gamepad(id);
                }
                SDL_free(ids.cast());
            }
            Ok(opened)
        }
    }
    fn open_gamepad(&mut self, id: u32) {
        if self
            .gamepads
            .iter()
            .flatten()
            .any(|&(other, _)| other == id)
        {
            return;
        }
        let Some(slot) = self.gamepads.iter_mut().find(|slot| slot.is_none()) else {
            return;
        };
        if let Some(pad) = NonNull::new(unsafe { SDL_OpenGamepad(id) }) {
            *slot = Some((id, pad));
        }
    }
    fn close_gamepad(&mut self, id: u32) {
        for slot in &mut self.gamepads {
            if slot.is_some_and(|(other, _)| other == id)
                && let Some((_, pad)) = slot.take()
            {
                unsafe {
                    SDL_CloseGamepad(pad.as_ptr());
                }
            }
        }
    }
    pub fn dropped_text_events(&self) -> u64 {
        self.dropped_text
    }
    fn drain_text(&mut self, queue: &mut SysEventQueue) {
        // Validated once at the SDL boundary, cursor always follows a char.
        for value in
            unsafe { std::str::from_utf8_unchecked(&self.text[self.text_cursor..self.text_len]) }
                .chars()
        {
            if queue.len() + 1 >= queue.capacity() {
                break;
            }
            if queue
                .push(SysEvent {
                    time: self.text_time,
                    kind: EventKind::Char {
                        device: DeviceId::Keyboard,
                        value,
                    },
                })
                .is_err()
            {
                break;
            }
            self.text_cursor += value.len_utf8();
        }
    }
    pub(crate) fn poll(&mut self, queue: &mut SysEventQueue, clock: &Clock) {
        self.drain_text(queue);
        if self.text_cursor < self.text_len {
            return;
        }
        let mut event = SdlEvent { padding: [0; 128] };
        while queue.len() + 1 < queue.capacity() && unsafe { SDL_PollEvent(&mut event) } {
            let time = clock.now();
            // SAFETY: SDL initializes the union variant corresponding to type.
            let kind = unsafe {
                match event.common.kind {
                    0x300 | 0x301 => {
                        let e = event.key;
                        if !(0..512).contains(&e.scancode) {
                            continue;
                        }
                        EventKind::Key {
                            device: DeviceId::Keyboard,
                            code: e.scancode as u16,
                            symbol: e.key as i32,
                            down: e.down,
                            repeat: e.repeat,
                        }
                    }
                    0x303 => {
                        let e = event.text;
                        if e.text.is_null() {
                            continue;
                        }
                        let bytes = CStr::from_ptr(e.text).to_bytes();
                        if bytes.len() > self.text.len() || std::str::from_utf8(bytes).is_err() {
                            self.dropped_text += 1;
                            continue;
                        }
                        self.text[..bytes.len()].copy_from_slice(bytes);
                        self.text_cursor = 0;
                        self.text_len = bytes.len();
                        self.text_time = time;
                        self.drain_text(queue);
                        if self.text_cursor < self.text_len {
                            break;
                        }
                        continue;
                    }
                    0x400 => {
                        let e = event.motion;
                        self.mouse_fraction[0] += e.dx;
                        self.mouse_fraction[1] += e.dy;
                        let dx = self.mouse_fraction[0] as i32;
                        let dy = self.mouse_fraction[1] as i32;
                        self.mouse_fraction[0] -= dx as f32;
                        self.mouse_fraction[1] -= dy as f32;
                        EventKind::Mouse {
                            device: DeviceId::Mouse(e.which),
                            dx,
                            dy,
                        }
                    }
                    0x401 | 0x402 => {
                        let e = event.button;
                        EventKind::MouseButton {
                            device: DeviceId::Mouse(e.which),
                            button: e.button,
                            down: e.down,
                        }
                    }
                    0x403 => {
                        let e = event.wheel;
                        let sign = if e.direction == 1 { -1 } else { 1 };
                        EventKind::MouseWheel {
                            device: DeviceId::Mouse(e.which),
                            x: e.integer_x.saturating_mul(sign),
                            y: e.integer_y.saturating_mul(sign),
                        }
                    }
                    0x650 => {
                        let e = event.axis;
                        EventKind::ControllerAxis {
                            device: DeviceId::Controller(e.which as i32),
                            axis: e.axis,
                            value: e.value,
                        }
                    }
                    0x651 | 0x652 => {
                        let e = event.pad_button;
                        EventKind::ControllerButton {
                            device: DeviceId::Controller(e.which as i32),
                            button: e.button,
                            down: e.down,
                        }
                    }
                    0x653 => {
                        self.open_gamepad(event.device.which);
                        continue;
                    }
                    0x654 => {
                        let id = event.device.which;
                        self.close_gamepad(id);
                        EventKind::DeviceRemoved(DeviceId::Controller(id as i32))
                    }
                    0x100 | 0x210 => EventKind::Quit,
                    0x20e => EventKind::Focus(true),
                    0x20f => {
                        self.mouse_fraction = [0.0; 2];
                        EventKind::Focus(false)
                    }
                    _ => continue,
                }
            };
            let _ = queue.push(SysEvent { time, kind });
        }
    }
    pub fn present(&mut self) {
        unsafe {
            SDL_RenderClear(self.renderer.as_ptr());
            SDL_RenderPresent(self.renderer.as_ptr());
        }
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            for (_, pad) in self.gamepads.iter().flatten() {
                SDL_CloseGamepad(pad.as_ptr());
            }
            SDL_DestroyRenderer(self.renderer.as_ptr());
            SDL_DestroyWindow(self.window.as_ptr());
            SDL_QuitSubSystem(SUBSYSTEMS);
        }
    }
}

#[cfg(all(test, target_pointer_width = "64"))]
mod abi_tests {
    use super::*;
    use std::mem::{offset_of, size_of};
    #[test]
    fn sdl3_header_layouts() {
        // SDL 3.4 headers, independently checked by the developer C ABI probe.
        assert_eq!(size_of::<SdlEvent>(), 128);
        assert_eq!((size_of::<SdlKey>(), offset_of!(SdlKey, down)), (40, 36));
        assert_eq!((size_of::<SdlText>(), offset_of!(SdlText, text)), (32, 24));
        assert_eq!(
            (size_of::<SdlMotion>(), offset_of!(SdlMotion, dx)),
            (48, 36)
        );
        assert_eq!(
            (size_of::<SdlButton>(), offset_of!(SdlButton, down)),
            (40, 25)
        );
        assert_eq!(
            (size_of::<SdlWheel>(), offset_of!(SdlWheel, integer_x)),
            (56, 44)
        );
        assert_eq!((size_of::<SdlAxis>(), offset_of!(SdlAxis, value)), (32, 24));
        assert_eq!(
            (size_of::<SdlPadButton>(), offset_of!(SdlPadButton, down)),
            (24, 21)
        );
        assert_eq!(
            (size_of::<SdlDevice>(), offset_of!(SdlDevice, which)),
            (24, 16)
        );
    }
}
