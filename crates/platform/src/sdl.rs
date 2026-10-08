use crate::clock::Clock;
use qa_core::sys_events::{DeviceId, EventKind, SysEvent, SysEventQueue};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr::NonNull;

#[repr(C, align(8))]
struct Event([u8; 56]);

#[link(name = "SDL2")]
unsafe extern "C" {
    fn SDL_Init(flags: u32) -> c_int;
    fn SDL_Quit();
    fn SDL_GetError() -> *const c_char;
    fn SDL_GetCurrentVideoDriver() -> *const c_char;
    fn SDL_CreateWindow(
        title: *const c_char,
        x: c_int,
        y: c_int,
        w: c_int,
        h: c_int,
        flags: u32,
    ) -> *mut c_void;
    fn SDL_DestroyWindow(window: *mut c_void);
    fn SDL_CreateRenderer(window: *mut c_void, index: c_int, flags: u32) -> *mut c_void;
    fn SDL_DestroyRenderer(renderer: *mut c_void);
    fn SDL_SetRenderDrawColor(renderer: *mut c_void, r: u8, g: u8, b: u8, a: u8) -> c_int;
    fn SDL_RenderClear(renderer: *mut c_void) -> c_int;
    fn SDL_RenderPresent(renderer: *mut c_void);
    fn SDL_PollEvent(event: *mut Event) -> c_int;
    fn SDL_StartTextInput();
    fn SDL_NumJoysticks() -> c_int;
    fn SDL_IsGameController(index: c_int) -> c_int;
    fn SDL_GameControllerOpen(index: c_int) -> *mut c_void;
    fn SDL_GameControllerClose(controller: *mut c_void);
    fn SDL_GameControllerGetJoystick(controller: *mut c_void) -> *mut c_void;
    fn SDL_JoystickInstanceID(joystick: *mut c_void) -> c_int;
    #[cfg(feature = "proof")]
    fn SDL_PushEvent(event: *mut Event) -> c_int;
    #[cfg(feature = "proof")]
    fn SDL_GetKeyFromName(name: *const c_char) -> c_int;
    #[cfg(feature = "proof")]
    fn SDL_GetScancodeFromKey(key: c_int) -> c_int;
}

fn error() -> String {
    unsafe { CStr::from_ptr(SDL_GetError()) }
        .to_string_lossy()
        .into_owned()
}

pub struct Window {
    window: NonNull<c_void>,
    renderer: NonNull<c_void>,
    controllers: [Option<(i32, NonNull<c_void>)>; 16],
}

impl Window {
    #[cfg(feature = "proof")]
    pub fn inject_key(&mut self, name: &str, down: bool) -> Result<(), String> {
        let name = std::ffi::CString::new(name).map_err(|_| "invalid key name")?;
        let key = unsafe { SDL_GetKeyFromName(name.as_ptr()) };
        if key == 0 {
            return Err("unknown key name".into());
        }
        let mut event = Event([0; 56]);
        event.0[..4].copy_from_slice(&(if down { 0x300u32 } else { 0x301u32 }).to_ne_bytes());
        event.0[12] = u8::from(down);
        event.0[16..20].copy_from_slice(&unsafe { SDL_GetScancodeFromKey(key) }.to_ne_bytes());
        event.0[20..24].copy_from_slice(&key.to_ne_bytes());
        if unsafe { SDL_PushEvent(&mut event) } != 1 {
            return Err(error());
        }
        Ok(())
    }

    #[cfg(feature = "proof")]
    pub fn inject_mouse(&mut self, dx: i32, dy: i32) -> Result<(), String> {
        let mut event = Event([0; 56]);
        event.0[..4].copy_from_slice(&0x400u32.to_ne_bytes());
        event.0[28..32].copy_from_slice(&dx.to_ne_bytes());
        event.0[32..36].copy_from_slice(&dy.to_ne_bytes());
        if unsafe { SDL_PushEvent(&mut event) } != 1 {
            return Err(error());
        }
        Ok(())
    }

    #[cfg(feature = "proof")]
    pub fn inject_text(&mut self, text: &str) -> Result<(), String> {
        let mut event = Event([0; 56]);
        if text.len() > 31 || text.contains('\0') {
            return Err("text input event exceeds 31 bytes".into());
        }
        event.0[..4].copy_from_slice(&0x303u32.to_ne_bytes());
        event.0[12..12 + text.len()].copy_from_slice(text.as_bytes());
        if unsafe { SDL_PushEvent(&mut event) } != 1 {
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
            if SDL_Init(0x20 | 0x2000) != 0 {
                return Err(error());
            }
            let Some(window) = NonNull::new(SDL_CreateWindow(
                c"Quake Anthology Rust".as_ptr(),
                0x2fff0000,
                0x2fff0000,
                width,
                height,
                4,
            )) else {
                let message = error();
                SDL_Quit();
                return Err(message);
            };
            let Some(renderer) = NonNull::new(SDL_CreateRenderer(window.as_ptr(), -1, 1)) else {
                let message = error();
                SDL_DestroyWindow(window.as_ptr());
                SDL_Quit();
                return Err(message);
            };
            SDL_SetRenderDrawColor(renderer.as_ptr(), 18, 26, 34, 255);
            SDL_StartTextInput();
            let mut opened = Self {
                window,
                renderer,
                controllers: [None; 16],
            };
            for index in 0..SDL_NumJoysticks() {
                opened.open_controller(index);
            }
            Ok(opened)
        }
    }

    fn open_controller(&mut self, index: i32) {
        let Some(slot) = self.controllers.iter_mut().find(|slot| slot.is_none()) else {
            return;
        };
        unsafe {
            if SDL_IsGameController(index) == 0 {
                return;
            }
            if let Some(controller) = NonNull::new(SDL_GameControllerOpen(index)) {
                let id = SDL_JoystickInstanceID(SDL_GameControllerGetJoystick(controller.as_ptr()));
                *slot = Some((id, controller));
            }
        }
    }
    fn close_controller(&mut self, id: i32) {
        for slot in &mut self.controllers {
            if slot.is_some_and(|(instance, _)| instance == id)
                && let Some((_, controller)) = slot.take()
            {
                unsafe {
                    SDL_GameControllerClose(controller.as_ptr());
                }
            }
        }
    }
    pub(crate) fn poll(&mut self, queue: &mut SysEventQueue, clock: &Clock) {
        let mut event = Event([0; 56]);
        // A UTF-8 SDL text event contains at most 31 characters. Leave room for
        // the complete event and the final time marker; retain other SDL input.
        while queue.len() + 32 < queue.capacity() && unsafe { SDL_PollEvent(&mut event) } != 0 {
            let word = |offset| {
                i32::from_ne_bytes([
                    event.0[offset],
                    event.0[offset + 1],
                    event.0[offset + 2],
                    event.0[offset + 3],
                ])
            };
            let time = clock.now();
            let kind = match word(0) as u32 {
                0x300 | 0x301 => {
                    let code = word(16);
                    if !(0..512).contains(&code) {
                        continue;
                    }
                    EventKind::Key {
                        device: DeviceId::Keyboard,
                        code: code as u16,
                        symbol: word(20),
                        down: word(0) == 0x300,
                        repeat: event.0[13] != 0,
                    }
                }
                0x303 => {
                    let text = &event.0[12..44];
                    let length = text
                        .iter()
                        .position(|&byte| byte == 0)
                        .unwrap_or(text.len());
                    if let Ok(text) = std::str::from_utf8(&text[..length]) {
                        for value in text.chars() {
                            let _ = queue.push(SysEvent {
                                time,
                                kind: EventKind::Char {
                                    device: DeviceId::Keyboard,
                                    value,
                                },
                            });
                        }
                    }
                    continue;
                }
                0x400 => EventKind::Mouse {
                    device: DeviceId::Mouse(word(12) as u32),
                    dx: word(28),
                    dy: word(32),
                },
                0x401 | 0x402 => EventKind::MouseButton {
                    device: DeviceId::Mouse(word(12) as u32),
                    button: event.0[16],
                    down: word(0) == 0x401,
                },
                0x403 => {
                    let direction = if word(24) == 1 { -1 } else { 1 };
                    EventKind::MouseWheel {
                        device: DeviceId::Mouse(word(12) as u32),
                        x: word(16).saturating_mul(direction),
                        y: word(20).saturating_mul(direction),
                    }
                }
                0x650 => EventKind::ControllerAxis {
                    device: DeviceId::Controller(word(8)),
                    axis: event.0[12],
                    value: i16::from_ne_bytes([event.0[16], event.0[17]]),
                },
                0x651 | 0x652 => EventKind::ControllerButton {
                    device: DeviceId::Controller(word(8)),
                    button: event.0[12],
                    down: word(0) == 0x651,
                },
                0x653 => {
                    self.open_controller(word(8));
                    continue;
                }
                0x654 => {
                    let id = word(8);
                    self.close_controller(id);
                    EventKind::DeviceRemoved(DeviceId::Controller(id))
                }
                0x100 => EventKind::Quit,
                0x200 => match event.0[12] {
                    12 => EventKind::Focus(true),
                    13 => EventKind::Focus(false),
                    14 => EventKind::Quit,
                    _ => continue,
                },
                _ => continue,
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
            for (_, controller) in self.controllers.iter().flatten() {
                SDL_GameControllerClose(controller.as_ptr());
            }
            SDL_DestroyRenderer(self.renderer.as_ptr());
            SDL_DestroyWindow(self.window.as_ptr());
            SDL_Quit();
        }
    }
}
