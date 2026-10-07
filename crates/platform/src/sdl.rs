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
            if SDL_Init(0x20) != 0 {
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
            Ok(Self { window, renderer })
        }
    }

    pub fn poll_quit(&mut self) -> bool {
        let mut event = Event([0; 56]);
        let mut quit = false;
        unsafe {
            while SDL_PollEvent(&mut event) != 0 {
                let kind = u32::from_ne_bytes([event.0[0], event.0[1], event.0[2], event.0[3]]);
                if kind == 0x300 {
                    println!("{{\"event\":\"key_down\",\"repeat\":{}}}", event.0[13] != 0);
                }
                if kind == 0x100 || (kind == 0x200 && event.0[12] == 14) {
                    quit = true;
                }
            }
        }
        quit
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
            SDL_DestroyRenderer(self.renderer.as_ptr());
            SDL_DestroyWindow(self.window.as_ptr());
            SDL_Quit();
        }
    }
}
