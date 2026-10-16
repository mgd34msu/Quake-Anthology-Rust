//! SDL3 ABI definitions follow SDL 3.4 headers, never SDL2 byte offsets.
use crate::clock::Clock;
use qa_core::sys_events::{
    DeviceId, EventKind, EventTime, MAX_INPUT_DEVICES, SysEvent, SysEventQueue,
};
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
#[derive(Clone, Copy, Default)]
struct SdlWindow {
    common: SdlCommon,
    window: u32,
    data1: i32,
    data2: i32,
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
    window: SdlWindow,
    padding: [u8; 128],
}

const SUBSYSTEMS: u32 = 0x20 | 0x2000; // VIDEO | GAMEPAD
const WINDOW_OPENGL: u64 = 0x2;
const WINDOW_INPUT_FOCUS: u64 = 0x200;
const PIXELFORMAT_ABGR8888: u32 = 0x16762004;
const TEXTUREACCESS_STREAMING: c_int = 1;
const GL_DOUBLEBUFFER: c_int = 5;
const GL_DEPTH_SIZE: c_int = 6;
const GL_CONTEXT_MAJOR_VERSION: c_int = 17;
const GL_CONTEXT_MINOR_VERSION: c_int = 18;
const GL_CONTEXT_PROFILE_MASK: c_int = 20;
const GL_CONTEXT_PROFILE_CORE: c_int = 1;
const MAX_WINDOW_DIMENSION: i32 = 8192;

// SDL_stdinc.h defines a nullable void (*)(void), not a data pointer.
type SdlFunctionPointer = Option<unsafe extern "C" fn()>;

#[repr(C)]
struct SdlRect {
    x: c_int,
    y: c_int,
    w: c_int,
    h: c_int,
}
#[repr(C)]
struct SdlFRect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

#[repr(C)]
struct SdlDisplayMode {
    display: u32,
    format: u32,
    width: c_int,
    height: c_int,
    pixel_density: f32,
    refresh_rate: f32,
    refresh_numerator: c_int,
    refresh_denominator: c_int,
    internal: *mut c_void,
}

#[link(name = "SDL3")]
unsafe extern "C" {
    fn SDL_InitSubSystem(flags: u32) -> bool;
    fn SDL_QuitSubSystem(flags: u32);
    fn SDL_GetError() -> *const c_char;
    fn SDL_GetCurrentVideoDriver() -> *const c_char;
    fn SDL_CreateWindow(title: *const c_char, w: c_int, h: c_int, flags: u64) -> *mut c_void;
    fn SDL_SyncWindow(window: *mut c_void) -> bool;
    fn SDL_GetWindowID(window: *mut c_void) -> u32;
    fn SDL_GetWindowFlags(window: *mut c_void) -> u64;
    fn SDL_GetWindowSizeInPixels(window: *mut c_void, w: *mut c_int, h: *mut c_int) -> bool;
    fn SDL_GetDisplayForWindow(window: *mut c_void) -> u32;
    fn SDL_GetCurrentDisplayMode(display: u32) -> *const SdlDisplayMode;
    fn SDL_SetWindowRelativeMouseMode(window: *mut c_void, enabled: bool) -> bool;
    fn SDL_GetWindowRelativeMouseMode(window: *mut c_void) -> bool;
    fn SDL_DestroyWindow(window: *mut c_void);
    fn SDL_CreateRenderer(window: *mut c_void, name: *const c_char) -> *mut c_void;
    fn SDL_DestroyRenderer(renderer: *mut c_void);
    fn SDL_SetRenderVSync(renderer: *mut c_void, vsync: c_int) -> bool;
    fn SDL_SetRenderDrawColor(renderer: *mut c_void, r: u8, g: u8, b: u8, a: u8) -> bool;
    fn SDL_RenderClear(renderer: *mut c_void) -> bool;
    fn SDL_RenderPresent(renderer: *mut c_void) -> bool;
    fn SDL_CreateTexture(
        renderer: *mut c_void,
        format: u32,
        access: c_int,
        w: c_int,
        h: c_int,
    ) -> *mut c_void;
    fn SDL_DestroyTexture(texture: *mut c_void);
    fn SDL_SetTextureBlendMode(texture: *mut c_void, mode: u32) -> bool;
    fn SDL_SetTextureScaleMode(texture: *mut c_void, mode: c_int) -> bool;
    fn SDL_UpdateTexture(
        texture: *mut c_void,
        rect: *const SdlRect,
        pixels: *const c_void,
        pitch: c_int,
    ) -> bool;
    fn SDL_RenderTexture(
        renderer: *mut c_void,
        texture: *mut c_void,
        source: *const SdlFRect,
        destination: *const SdlFRect,
    ) -> bool;
    fn SDL_GL_SetAttribute(attribute: c_int, value: c_int) -> bool;
    fn SDL_GL_GetAttribute(attribute: c_int, value: *mut c_int) -> bool;
    fn SDL_GL_CreateContext(window: *mut c_void) -> *mut c_void;
    fn SDL_GL_DestroyContext(context: *mut c_void) -> bool;
    fn SDL_GL_GetCurrentContext() -> *mut c_void;
    fn SDL_GL_MakeCurrent(window: *mut c_void, context: *mut c_void) -> bool;
    fn SDL_GL_GetProcAddress(name: *const c_char) -> SdlFunctionPointer;
    fn SDL_GL_SetSwapInterval(interval: c_int) -> bool;
    fn SDL_GL_GetSwapInterval(interval: *mut c_int) -> bool;
    fn SDL_GL_SwapWindow(window: *mut c_void) -> bool;
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

enum Presentation {
    Software {
        renderer: NonNull<c_void>,
        texture: NonNull<c_void>,
        width: u32,
        height: u32,
    },
    Gl {
        context: NonNull<c_void>,
    },
}
impl Presentation {
    fn software(window: NonNull<c_void>, width: i32, height: i32) -> Result<Self, String> {
        // All resources are created on the SDL main thread and stay with Window.
        unsafe {
            let renderer = NonNull::new(SDL_CreateRenderer(window.as_ptr(), c"software".as_ptr()))
                .ok_or_else(error)?;
            let Some(texture) = NonNull::new(SDL_CreateTexture(
                renderer.as_ptr(),
                PIXELFORMAT_ABGR8888,
                TEXTUREACCESS_STREAMING,
                width,
                height,
            )) else {
                let message = error();
                SDL_DestroyRenderer(renderer.as_ptr());
                return Err(message);
            };
            // Pixels already include the scene blend. Copy them without a
            // second alpha blend, and preserve native pixel edges if scaled.
            if !SDL_SetTextureBlendMode(texture.as_ptr(), 0)
                || !SDL_SetTextureScaleMode(texture.as_ptr(), 0)
                || !SDL_SetRenderVSync(renderer.as_ptr(), 0)
                || !SDL_SetRenderDrawColor(renderer.as_ptr(), 18, 26, 34, 255)
            {
                let message = error();
                SDL_DestroyTexture(texture.as_ptr());
                SDL_DestroyRenderer(renderer.as_ptr());
                return Err(message);
            }
            Ok(Self::Software {
                renderer,
                texture,
                width: width as u32,
                height: height as u32,
            })
        }
    }
    fn gl(window: NonNull<c_void>) -> Result<Self, String> {
        unsafe {
            let context = NonNull::new(SDL_GL_CreateContext(window.as_ptr())).ok_or_else(error)?;
            let result = (|| {
                if !SDL_GL_MakeCurrent(window.as_ptr(), context.as_ptr()) {
                    return Err(error());
                }
                let (mut major, mut minor, mut profile) = (0, 0, 0);
                if !SDL_GL_GetAttribute(GL_CONTEXT_MAJOR_VERSION, &mut major)
                    || !SDL_GL_GetAttribute(GL_CONTEXT_MINOR_VERSION, &mut minor)
                    || !SDL_GL_GetAttribute(GL_CONTEXT_PROFILE_MASK, &mut profile)
                {
                    return Err(error());
                }
                if (major, minor) < (4, 4) || profile & GL_CONTEXT_PROFILE_CORE == 0 {
                    return Err("OpenGL 4.4 core context required".into());
                }
                let mut interval = -1;
                // Xvfb GLX has no swap-control extension: setting zero returns
                // unsupported, but SDL's GLX getter reports its zero interval.
                // Accept the requested state only after a successful query.
                SDL_GL_SetSwapInterval(0);
                if !SDL_GL_GetSwapInterval(&mut interval) {
                    return Err(error());
                }
                if interval != 0 {
                    return Err("OpenGL swap interval must be zero".into());
                }
                Ok(())
            })();
            if let Err(message) = result {
                SDL_GL_DestroyContext(context.as_ptr());
                return Err(message);
            }
            Ok(Self::Gl { context })
        }
    }
}

pub(crate) fn error() -> String {
    unsafe { CStr::from_ptr(SDL_GetError()) }
        .to_string_lossy()
        .into_owned()
}

#[derive(Clone, Copy)]
struct MouseRemainder {
    id: u32,
    fraction: [f32; 2],
}

fn synchronized_capture(
    previous: bool,
    desired: bool,
    mut set: impl FnMut(bool) -> bool,
    mut sync: impl FnMut() -> bool,
) -> bool {
    if !set(desired) {
        return false;
    }
    if sync() {
        return true;
    }
    set(previous);
    sync();
    false
}

/// SDL conversion state only: no queued input or borrowed text survives a poll.
struct SdlInput {
    window: u32,
    focused: bool,
    handoff_pending: bool,
    capture_requested: bool,
    relative: bool,
    mice: [Option<MouseRemainder>; MAX_INPUT_DEVICES],
    dropped_text: u64,
    dropped_mouse: u64,
    capture_errors: u64,
}
impl SdlInput {
    fn new(window: u32, focused: bool) -> Self {
        Self {
            window,
            focused,
            handoff_pending: false,
            capture_requested: false,
            relative: false,
            mice: [None; MAX_INPUT_DEVICES],
            dropped_text: 0,
            dropped_mouse: 0,
            capture_errors: 0,
        }
    }
    fn publish_handoff(&mut self, queue: &mut SysEventQueue, time: EventTime) -> bool {
        if !self.handoff_pending {
            return true;
        }
        if !queue.has_ordinary_room()
            || queue
                .push(SysEvent {
                    time,
                    kind: EventKind::Focus(false),
                })
                .is_err()
        {
            return false;
        }
        self.handoff_pending = false;
        true
    }
    fn reconcile_capture(
        &mut self,
        queue: &mut SysEventQueue,
        time: EventTime,
        notify: bool,
        apply: &mut impl FnMut(bool) -> bool,
    ) {
        let desired = self.capture_requested && self.focused;
        if desired == self.relative || notify && !queue.has_ordinary_room() {
            return;
        }
        if !apply(desired) {
            self.capture_errors = self.capture_errors.saturating_add(1);
            return;
        }
        self.relative = desired;
        self.mice.fill(None);
        // Focus(false) already releases every device and its filter history.
        if notify
            && queue
                .push(SysEvent {
                    time,
                    kind: EventKind::MouseCapture(desired),
                })
                .is_err()
        {
            self.capture_errors = self.capture_errors.saturating_add(1);
        }
    }
    fn motion(&mut self, id: u32, delta: [f32; 2]) -> Option<[i32; 2]> {
        if !delta.iter().all(|value| value.is_finite()) {
            self.dropped_mouse = self.dropped_mouse.saturating_add(1);
            return None;
        }
        let index = self
            .mice
            .iter()
            .position(|slot| slot.is_some_and(|mouse| mouse.id == id))
            .or_else(|| self.mice.iter().position(Option::is_none));
        let Some(index) = index else {
            self.dropped_mouse = self.dropped_mouse.saturating_add(1);
            return None;
        };
        let mouse = self.mice[index].get_or_insert(MouseRemainder {
            id,
            fraction: [0.0; 2],
        });
        Some(std::array::from_fn(|axis| {
            mouse.fraction[axis] += delta[axis];
            let integer = mouse.fraction[axis] as i32;
            mouse.fraction[axis] -= integer as f32;
            integer
        }))
    }
    /// SDL must initialize the tagged union and keep its text pointer alive
    /// until queue admission has copied the returned payload.
    unsafe fn decode<'a>(&mut self, event: &'a SdlEvent) -> Option<EventKind<'a>> {
        // SAFETY: the caller supplies a correctly tagged SDL event, including
        // the text pointer's validity through the immediately following push.
        let kind = unsafe {
            match event.common.kind {
                0x300 | 0x301 => {
                    let e = event.key;
                    if e.window != self.window || !self.focused || !(0..512).contains(&e.scancode) {
                        return None;
                    }
                    EventKind::Key {
                        device: DeviceId::Keyboard(e.which),
                        code: e.scancode as u16,
                        symbol: e.key as i32,
                        down: e.down,
                        repeat: e.repeat,
                    }
                }
                0x303 => {
                    let e = event.text;
                    if e.window != self.window || !self.focused || e.text.is_null() {
                        return None;
                    }
                    let Ok(text) = CStr::from_ptr(e.text).to_str() else {
                        self.dropped_text = self.dropped_text.saturating_add(1);
                        return None;
                    };
                    // SDL text/IME has a window but no physical keyboard ID.
                    EventKind::Text {
                        device: DeviceId::Keyboard(0),
                        text,
                    }
                }
                0x400 => {
                    let e = event.motion;
                    if e.window != self.window || !self.focused || !self.relative {
                        return None;
                    }
                    let [dx, dy] = self.motion(e.which, [e.dx, e.dy])?;
                    EventKind::Mouse {
                        device: DeviceId::Mouse(e.which),
                        dx,
                        dy,
                    }
                }
                0x401 | 0x402 => {
                    let e = event.button;
                    if e.window != self.window || !self.focused || !self.relative {
                        return None;
                    }
                    EventKind::MouseButton {
                        device: DeviceId::Mouse(e.which),
                        button: e.button,
                        down: e.down,
                    }
                }
                0x403 => {
                    let e = event.wheel;
                    if e.window != self.window || !self.focused || !self.relative {
                        return None;
                    }
                    let sign = if e.direction == 1 { -1 } else { 1 };
                    EventKind::MouseWheel {
                        device: DeviceId::Mouse(e.which),
                        x: e.integer_x.saturating_mul(sign),
                        y: e.integer_y.saturating_mul(sign),
                    }
                }
                0x650 if self.focused => {
                    let e = event.axis;
                    EventKind::ControllerAxis {
                        device: DeviceId::Controller(e.which as i32),
                        axis: e.axis,
                        value: e.value,
                    }
                }
                0x651 | 0x652 if self.focused => {
                    let e = event.pad_button;
                    EventKind::ControllerButton {
                        device: DeviceId::Controller(e.which as i32),
                        button: e.button,
                        down: e.down,
                    }
                }
                0x306 => EventKind::DeviceRemoved(DeviceId::Keyboard(event.device.which)),
                0x405 => {
                    let id = event.device.which;
                    for slot in &mut self.mice {
                        if slot.is_some_and(|mouse| mouse.id == id) {
                            *slot = None;
                        }
                    }
                    EventKind::DeviceRemoved(DeviceId::Mouse(id))
                }
                0x654 => EventKind::DeviceRemoved(DeviceId::Controller(event.device.which as i32)),
                0x100 => EventKind::Quit,
                0x210 if event.window.window == self.window => EventKind::Quit,
                0x20e | 0x20f if event.window.window == self.window => {
                    self.focused = event.common.kind == 0x20e;
                    if !self.focused {
                        self.mice.fill(None);
                    }
                    EventKind::Focus(self.focused)
                }
                _ => return None,
            }
        };
        Some(kind)
    }
}

pub struct Window {
    window: NonNull<c_void>,
    presentation: Presentation,
    gamepads: [Option<(u32, NonNull<c_void>)>; 16],
    input: SdlInput,
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
            window: self.input.window,
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
            window: self.input.window,
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
            window: self.input.window,
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
    /// Actual render-target and display properties. A software presenter uses
    /// its fixed framebuffer, which SDL may scale to a different window size.
    pub fn display_configuration(
        &self,
    ) -> Result<qa_core::presentation::DisplayConfiguration, String> {
        let mut result = qa_core::presentation::DisplayConfiguration::default();
        // SAFETY: the owned SDL window and current GL context remain live on
        // this thread; SDL owns the returned display-mode record.
        unsafe {
            result.fullscreen = SDL_GetWindowFlags(self.window.as_ptr()) & 1 != 0;
            let display = SDL_GetDisplayForWindow(self.window.as_ptr());
            if let Some(mode) = SDL_GetCurrentDisplayMode(display).as_ref() {
                result.frequency = mode.refresh_rate as i32;
            }
            match self.presentation {
                Presentation::Software { width, height, .. } => {
                    result.width = width;
                    result.height = height;
                    result.color_bits = 24;
                    result.depth_bits = 32;
                }
                Presentation::Gl { .. } => {
                    let (mut width, mut height) = (0, 0);
                    if !SDL_GetWindowSizeInPixels(self.window.as_ptr(), &mut width, &mut height)
                        || width <= 0
                        || height <= 0
                    {
                        return Err(error());
                    }
                    result.width = width as u32;
                    result.height = height as u32;
                    let mut attributes = [0; 6];
                    for (target, attribute) in attributes.iter_mut().zip([0, 1, 2, 6, 7, 12]) {
                        if !SDL_GL_GetAttribute(attribute, target) || *target < 0 {
                            return Err(error());
                        }
                    }
                    result.color_bits = (attributes[0] + attributes[1] + attributes[2]) as u32;
                    result.depth_bits = attributes[3] as u32;
                    result.stencil_bits = attributes[4] as u32;
                    result.stereo = attributes[5] != 0;
                }
            }
        }
        Ok(result)
    }
    /// Opens the SDL software presenter with a fixed streaming framebuffer.
    /// SDL window operations must remain on the main thread that creates it.
    pub fn open(width: i32, height: i32) -> Result<Self, String> {
        Self::open_backend(width, height, false)
    }
    /// Opens a double-buffered OpenGL 4.4 core context with vsync disabled.
    /// The context is current on return. Window is not Send or Sync; GL draws,
    /// procedure resolution and presentation stay on its creating main thread.
    pub fn open_gl(width: i32, height: i32) -> Result<Self, String> {
        Self::open_backend(width, height, true)
    }
    fn open_backend(width: i32, height: i32, gl: bool) -> Result<Self, String> {
        if !(1..=MAX_WINDOW_DIMENSION).contains(&width)
            || !(1..=MAX_WINDOW_DIMENSION).contains(&height)
        {
            return Err("window dimensions must be between 1 and 8192".into());
        }
        unsafe {
            if !SDL_InitSubSystem(SUBSYSTEMS) {
                return Err(error());
            }
            if gl {
                for (attribute, value) in [
                    (GL_CONTEXT_MAJOR_VERSION, 4),
                    (GL_CONTEXT_MINOR_VERSION, 4),
                    (GL_CONTEXT_PROFILE_MASK, GL_CONTEXT_PROFILE_CORE),
                    (GL_DOUBLEBUFFER, 1),
                    (GL_DEPTH_SIZE, 24),
                ] {
                    if !SDL_GL_SetAttribute(attribute, value) {
                        let message = error();
                        SDL_QuitSubSystem(SUBSYSTEMS);
                        return Err(message);
                    }
                }
            }
            let Some(window) = NonNull::new(SDL_CreateWindow(
                c"Quake Anthology Rust".as_ptr(),
                width,
                height,
                if gl { WINDOW_OPENGL } else { 0 },
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
            let presentation = match if gl {
                Presentation::gl(window)
            } else {
                Presentation::software(window, width, height)
            } {
                Ok(presentation) => presentation,
                Err(message) => {
                    SDL_DestroyWindow(window.as_ptr());
                    SDL_QuitSubSystem(SUBSYSTEMS);
                    return Err(message);
                }
            };
            SDL_StartTextInput(window.as_ptr());
            let mut opened = Self {
                window,
                presentation,
                gamepads: [None; 16],
                input: SdlInput::new(
                    SDL_GetWindowID(window.as_ptr()),
                    SDL_GetWindowFlags(window.as_ptr()) & WINDOW_INPUT_FOCUS != 0,
                ),
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
        self.input.dropped_text
    }
    pub fn dropped_mouse_events(&self) -> u64 {
        self.input.dropped_mouse
    }
    pub fn capture_errors(&self) -> u64 {
        self.input.capture_errors
    }
    /// Selection only; SDL state changes occur inside the two physical intakes.
    pub fn request_mouse_capture(&mut self, requested: bool) {
        self.input.capture_requested = requested;
    }
    /// Release the previous window's held input at the next ordinary intake.
    pub fn handoff_input(&mut self) {
        self.input.handoff_pending = true;
    }
    pub(crate) fn poll(&mut self, queue: &mut SysEventQueue, clock: &Clock) {
        if self.input.handoff_pending && !self.input.publish_handoff(queue, clock.now()) {
            return;
        }
        let window = self.window.as_ptr();
        // SAFETY: the owned SDL window remains on its creating thread. Failed
        // synchronization rolls back capture instead of claiming completion.
        let mut apply_capture = |enabled| unsafe {
            synchronized_capture(
                SDL_GetWindowRelativeMouseMode(window),
                enabled,
                |mode| SDL_SetWindowRelativeMouseMode(window, mode),
                || SDL_SyncWindow(window),
            )
        };
        self.input
            .reconcile_capture(queue, clock.now(), true, &mut apply_capture);
        let mut event = SdlEvent { padding: [0; 128] };
        while queue.has_ordinary_room() && unsafe { SDL_PollEvent(&mut event) } {
            let time = clock.now();
            // SAFETY: SDL initializes the union variant corresponding to type.
            match unsafe { event.common.kind } {
                0x653 => {
                    self.open_gamepad(unsafe { event.device.which });
                    continue;
                }
                0x654 => self.close_gamepad(unsafe { event.device.which }),
                _ => {}
            }
            // SDL's text bytes stay valid until the next SDL_PollEvent. Queue
            // admission copies them here and no pointer survives that poll.
            let Some(kind) = (unsafe { self.input.decode(&event) }) else {
                continue;
            };
            if queue.push(SysEvent { time, kind }).is_err() {
                if matches!(kind, EventKind::Text { .. }) {
                    self.input.dropped_text = self.input.dropped_text.saturating_add(1);
                }
                continue;
            }
            self.input.reconcile_capture(
                queue,
                time,
                !matches!(kind, EventKind::Focus(false)),
                &mut apply_capture,
            );
        }
    }
    pub fn present(&mut self) {
        match &self.presentation {
            Presentation::Software { renderer, .. } => unsafe {
                SDL_RenderClear(renderer.as_ptr());
                SDL_RenderPresent(renderer.as_ptr());
            },
            Presentation::Gl { .. } => {
                self.present_gl();
            }
        }
    }
    /// Restores this window's GL context on its creating main thread.
    /// Software windows need no context switch.
    pub fn ensure_gl_current(&self) -> bool {
        let Presentation::Gl { context } = &self.presentation else {
            return true;
        };
        // SAFETY: Window retains this context and its creating thread owns SDL.
        unsafe {
            SDL_GL_GetCurrentContext() == context.as_ptr()
                || SDL_GL_MakeCurrent(self.window.as_ptr(), context.as_ptr())
        }
    }
    /// Resolves a GL function for this window's current context during loading.
    /// Returns null on a software window, failed context switch or missing name.
    /// The caller must use the function's GL ABI and keep it within this context's
    /// lifetime. Calling the resulting function pointer is unsafe.
    pub fn gl_proc(&self, name: &CStr) -> *const c_void {
        let Presentation::Gl { .. } = &self.presentation else {
            return std::ptr::null();
        };
        if !self.ensure_gl_current() {
            return std::ptr::null();
        }
        unsafe {
            match SDL_GL_GetProcAddress(name.as_ptr()) {
                Some(function) => function as *const () as *const c_void,
                None => std::ptr::null(),
            }
        }
    }
    /// Swaps this window's GL buffers on its creating main thread.
    pub fn present_gl(&mut self) -> bool {
        let Presentation::Gl { .. } = &self.presentation else {
            return false;
        };
        self.ensure_gl_current() && unsafe { SDL_GL_SwapWindow(self.window.as_ptr()) }
    }
    /// Copies an exact-size ABGR8888 framebuffer to the fixed software texture.
    /// On little-endian hosts each u32 stores R, G, B, A bytes in that order.
    /// Size or backend mismatches return false without resizing or allocating.
    pub fn present_pixels(&mut self, width: u32, height: u32, pixels: &[u32]) -> bool {
        let Presentation::Software {
            renderer,
            texture,
            width: expected_width,
            height: expected_height,
        } = &self.presentation
        else {
            return false;
        };
        if width != *expected_width
            || height != *expected_height
            || pixels.len() != width as usize * height as usize
        {
            return false;
        }
        unsafe {
            SDL_UpdateTexture(
                texture.as_ptr(),
                std::ptr::null(),
                pixels.as_ptr().cast(),
                (width * 4) as c_int,
            ) && SDL_RenderTexture(
                renderer.as_ptr(),
                texture.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
            ) && SDL_RenderPresent(renderer.as_ptr())
        }
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            if self.input.relative {
                SDL_SetWindowRelativeMouseMode(self.window.as_ptr(), false);
                SDL_SyncWindow(self.window.as_ptr());
            }
            for (_, pad) in self.gamepads.iter().flatten() {
                SDL_CloseGamepad(pad.as_ptr());
            }
            match &self.presentation {
                Presentation::Software {
                    renderer, texture, ..
                } => {
                    SDL_DestroyTexture(texture.as_ptr());
                    SDL_DestroyRenderer(renderer.as_ptr());
                }
                Presentation::Gl { context } => {
                    SDL_GL_DestroyContext(context.as_ptr());
                }
            }
            SDL_DestroyWindow(self.window.as_ptr());
            SDL_QuitSubSystem(SUBSYSTEMS);
        }
    }
}

#[cfg(test)]
#[path = "../tests/sdl/events.rs"]
mod input_tests;

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
