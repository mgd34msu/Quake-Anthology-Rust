//! SDL OpenGL context handoff between the main thread and a render worker.
//!
//! Port of donor `src/platform/sdl-render-context.ts` (the SDL replacement
//! for `GLimp_*` SMP handoff). The window owner detaches its context into a
//! [`SdlRenderContextTransfer`] token; a worker adopts it into an
//! [`SdlWorkerRenderContext`]. Native pointers stay in SDL's window data.

use std::ffi::c_void;
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;

use crate::error::{Error, Result};
use crate::ffi_util::{c_string, sdl_error, LoadedLibrary};
use crate::native_libraries::{NativeLibrary, NativeLibraryOptions};

// Transfer states, mirroring the donor's ownership cell values.
const OFFERED: i32 = 0;
const ADOPTED: i32 = 1;
const RELEASED: i32 = 2;
const RESTORING: i32 = 3;
const RESTORED: i32 = 4;
const PARKED: i32 = 5;

/// Parked ownership state, for window-list reservation checks.
pub(crate) const TRANSFER_PARKED: i32 = PARKED;

/// Released when a GL procedure table closes; keeps the owning context alive.
pub struct ProcedureGuard {
    counter: Arc<AtomicUsize>,
    released: bool,
}

impl ProcedureGuard {
    pub(crate) fn new(counter: Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self {
            counter,
            released: false,
        }
    }

    /// Release the lease early. Dropping releases automatically.
    pub fn release(mut self) {
        self.release_inner();
        self.released = true;
    }

    fn release_inner(&mut self) {
        if !self.released {
            self.released = true;
            self.counter.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

impl Drop for ProcedureGuard {
    fn drop(&mut self) {
        self.release_inner();
    }
}

/// Shared GL context surface used by the GL procedure-table loaders.
pub trait SdlRenderContext {
    /// Current drawable size in pixels.
    fn drawable_size(&mut self) -> Result<(u32, u32)>;
    /// Whether rendering calls reach the context (diagnostic switch).
    fn rendering_enabled(&self) -> bool;
    /// Attach or detach the context for diagnostics.
    fn set_rendering_enabled(&mut self, enabled: bool) -> Result<()>;
    /// Make the context current on this thread.
    fn make_current(&mut self) -> Result<()>;
    /// Resolve one GL entry point through the current context.
    fn get_gl_proc_address(&mut self, name: &str) -> Result<*mut c_void>;
    /// Hold the context alive for a GL procedure table.
    fn retain_procedures(&mut self) -> Result<ProcedureGuard>;
    /// Swap front/back buffers.
    fn swap(&mut self) -> Result<()>;
}

/// Same-process worker token. Native pointers stay in SDL's window data.
#[derive(Clone)]
pub struct SdlRenderContextTransfer {
    /// SDL window id the context belongs to.
    pub window_id: u32,
    /// `quake-render-<uuid>` window-data key.
    pub key: String,
    /// Shared ownership cell.
    pub state: Arc<AtomicI32>,
}

macro_rules! render_sdl_symbols {
    ($($field:ident : $cname:literal : $sig:ty;)*) => {
        /// Loaded SDL subset for render-context handoff.
        pub struct RenderSdl {
            _lib: LoadedLibrary,
            $(pub(crate) $field: $sig,)*
        }
        impl RenderSdl {
            /// # Safety
            ///
            /// Resolved symbols are only invoked with the SDL ABI below.
            pub unsafe fn load(options: &NativeLibraryOptions) -> Result<Arc<Self>> {
                // SAFETY: loading maps the image without invoking its code.
                let lib = unsafe { LoadedLibrary::open(NativeLibrary::Sdl2, options)? };
                $(let $field: $sig = unsafe { lib.symbol(concat!($cname, "\0").as_bytes())? };)*
                Ok(Arc::new(Self { _lib: lib, $($field,)* }))
            }
        }
    };
}

render_sdl_symbols! {
    sdl_get_error: "SDL_GetError": unsafe extern "C" fn() -> *const u8;
    sdl_get_window_from_id: "SDL_GetWindowFromID": unsafe extern "C" fn(u32) -> *mut c_void;
    sdl_set_window_data: "SDL_SetWindowData": unsafe extern "C" fn(*mut c_void, *const u8, *mut c_void) -> *mut c_void;
    sdl_get_window_data: "SDL_GetWindowData": unsafe extern "C" fn(*mut c_void, *const u8) -> *mut c_void;
    sdl_gl_make_current: "SDL_GL_MakeCurrent": unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32;
    sdl_gl_get_current_context: "SDL_GL_GetCurrentContext": unsafe extern "C" fn() -> *mut c_void;
    sdl_gl_get_drawable_size: "SDL_GL_GetDrawableSize": unsafe extern "C" fn(*mut c_void, *mut i32, *mut i32);
    sdl_gl_get_proc_address: "SDL_GL_GetProcAddress": unsafe extern "C" fn(*const u8) -> *mut c_void;
    sdl_gl_swap_window: "SDL_GL_SwapWindow": unsafe extern "C" fn(*mut c_void);
    sdl_gl_set_swap_interval: "SDL_GL_SetSwapInterval": unsafe extern "C" fn(i32) -> i32;
    sdl_gl_get_swap_interval: "SDL_GL_GetSwapInterval": unsafe extern "C" fn() -> i32;
}

impl RenderSdl {
    /// # Safety
    ///
    /// Every argument must satisfy the corresponding SDL contract.
    unsafe fn error(&self) -> String {
        // SAFETY: SDL guarantees a valid thread-local error string.
        unsafe { sdl_error(self.sdl_get_error) }
    }

    /// # Safety
    ///
    /// Caller must pass SDL's return for `operation`.
    unsafe fn checked(&self, result: i32, operation: &str) -> Result<()> {
        if result < 0 {
            // SAFETY: error read immediately after the failing call.
            return Err(Error::native(operation, unsafe { self.error() }));
        }
        Ok(())
    }
}

/// Validate a transfer token the way the donor validates unknown input.
pub fn validate_transfer(window_id: u32, key: &str) -> Result<()> {
    if window_id == 0 {
        return Err(Error::InvalidInput("invalid SDL render context transfer".to_string()));
    }
    if !is_transfer_key(key) {
        return Err(Error::InvalidInput("invalid SDL render context transfer".to_string()));
    }
    Ok(())
}

fn is_transfer_key(key: &str) -> bool {
    let rest = key.strip_prefix("quake-render-");
    let Some(rest) = rest else {
        return false;
    };
    if rest.len() != 36 {
        return false;
    }
    for (index, byte) in rest.bytes().enumerate() {
        let dash = index == 8 || index == 13 || index == 18 || index == 23;
        if dash {
            if byte != b'-' {
                return false;
            }
        } else if !byte.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

pub(crate) fn random_transfer_key() -> String {
    format!("quake-render-{}", random_uuid())
}

fn random_uuid() -> String {
    let mut bytes = [0u8; 16];
    if read_random(&mut bytes).is_err() {
        // Fallback: mix pid, time, and address entropy (version nibble still 4).
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        use std::time::{SystemTime, UNIX_EPOCH};
        let mut hasher = DefaultHasher::new();
        std::process::id().hash(&mut hasher);
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
            .hash(&mut hasher);
        thread::current().id().hash(&mut hasher);
        let a = hasher.finish();
        let mut hasher = DefaultHasher::new();
        a.hash(&mut hasher);
        bytes.as_ptr().hash(&mut hasher);
        let b = hasher.finish();
        bytes[0..8].copy_from_slice(&a.to_le_bytes());
        bytes[8..16].copy_from_slice(&b.to_le_bytes());
    }
    bytes[6] = bytes[6] & 0x0f | 0x40;
    bytes[8] = bytes[8] & 0x3f | 0x80;
    let mut out = String::with_capacity(36);
    for (index, byte) in bytes.iter().enumerate() {
        if index == 4 || index == 6 || index == 8 || index == 10 {
            out.push('-');
        }
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(unix)]
fn read_random(bytes: &mut [u8]) -> std::io::Result<()> {
    use std::io::Read;
    std::fs::File::open("/dev/urandom")?.read_exact(bytes)
}

#[cfg(not(unix))]
fn read_random(_: &mut [u8]) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no entropy source",
    ))
}

/// The window owner keeps this lease until cancellation or worker release.
pub struct SdlRenderContextLease {
    window: *mut c_void,
    context: *mut c_void,
    sdl: Arc<RenderSdl>,
    transfer: SdlRenderContextTransfer,
    owner: thread::ThreadId,
}

// SAFETY: the lease is only touched on the detaching thread; the state cell is atomic.
unsafe impl Send for SdlRenderContextLease {}

impl SdlRenderContextLease {
    /// Detach a current context into a worker token.
    ///
    /// # Safety
    ///
    /// `window`/`context` must be a live SDL window and its current GL
    /// context on this thread.
    pub unsafe fn detach(
        window: *mut c_void,
        context: *mut c_void,
        window_id: u32,
        sdl: Arc<RenderSdl>,
        owner: thread::ThreadId,
    ) -> Result<Self> {
        if thread::current().id() != owner {
            return Err(Error::InvalidInput(
                "SDL window lifetime belongs to the owning thread".to_string(),
            ));
        }
        if cfg!(target_os = "macos") {
            return Err(Error::Unsupported(
                "SDL OpenGL render workers require main-queue dispatch support on macOS; use the serial renderer"
                    .to_string(),
            ));
        }
        // SAFETY: SDL calls below use validated handles.
        unsafe {
            if (sdl.sdl_gl_get_current_context)() != context {
                return Err(Error::InvalidInput(
                    "SDL context must be current before detach".to_string(),
                ));
            }
            let transfer = SdlRenderContextTransfer {
                window_id,
                key: random_transfer_key(),
                state: Arc::new(AtomicI32::new(OFFERED)),
            };
            validate_transfer(transfer.window_id, &transfer.key)?;
            let key = c_string(&transfer.key)?;
            let owner_key = c_string(&format!("{}-owner", transfer.key))?;
            let token = Arc::as_ptr(&transfer.state) as *mut c_void;
            (sdl.sdl_set_window_data)(window, key.as_ptr(), context);
            (sdl.sdl_set_window_data)(window, owner_key.as_ptr(), token);
            if (sdl.sdl_get_window_data)(window, key.as_ptr()) != context
                || (sdl.sdl_get_window_data)(window, owner_key.as_ptr()) != token
            {
                (sdl.sdl_set_window_data)(window, key.as_ptr(), std::ptr::null_mut());
                (sdl.sdl_set_window_data)(window, owner_key.as_ptr(), std::ptr::null_mut());
                return Err(Error::native(
                    "SDL_SetWindowData",
                    "SDL context transfer registration failed".to_string(),
                ));
            }
            if let Err(error) = sdl.checked(
                (sdl.sdl_gl_make_current)(window, std::ptr::null_mut()),
                "SDL_GL_MakeCurrent detach",
            ) {
                (sdl.sdl_set_window_data)(window, key.as_ptr(), std::ptr::null_mut());
                (sdl.sdl_set_window_data)(window, owner_key.as_ptr(), std::ptr::null_mut());
                return Err(error);
            }
            transfer.state.store(OFFERED, Ordering::SeqCst);
            Ok(Self {
                window,
                context,
                sdl,
                transfer,
                owner,
            })
        }
    }

    /// Transfer token handed to the worker.
    #[must_use]
    pub fn transfer(&self) -> SdlRenderContextTransfer {
        self.transfer.clone()
    }

    /// Whether the worker parked the context (window-list operations allowed).
    #[must_use]
    pub fn parked(&self) -> bool {
        self.transfer.state.load(Ordering::SeqCst) == PARKED
    }

    /// Shared ownership cell, for window-list reservation checks.
    #[must_use]
    pub fn state(&self) -> Arc<AtomicI32> {
        Arc::clone(&self.transfer.state)
    }

    /// Cancel an unadopted token, or restore after the worker cleared current.
    pub fn restore(&self) -> Result<()> {
        if thread::current().id() != self.owner {
            return Err(Error::InvalidInput(
                "SDL context restoration belongs to the owning thread".to_string(),
            ));
        }
        let current = self.transfer.state.load(Ordering::SeqCst);
        if current == RESTORED {
            return Ok(());
        }
        if (current != OFFERED && current != RELEASED)
            || self
                .transfer
                .state
                .compare_exchange(current, RESTORING, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
        {
            return Err(Error::InvalidInput(
                "SDL render context is still owned by the worker".to_string(),
            ));
        }
        // SAFETY: the window and context are live; the worker released current.
        unsafe {
            if let Err(error) = self.sdl.checked(
                (self.sdl.sdl_gl_make_current)(self.window, self.context),
                "SDL_GL_MakeCurrent restore",
            ) {
                self.transfer.state.store(RELEASED, Ordering::SeqCst);
                return Err(error);
            }
            let key = c_string(&self.transfer.key)?;
            let owner_key = c_string(&format!("{}-owner", self.transfer.key))?;
            (self.sdl.sdl_set_window_data)(self.window, key.as_ptr(), std::ptr::null_mut());
            (self.sdl.sdl_set_window_data)(self.window, owner_key.as_ptr(), std::ptr::null_mut());
        }
        self.transfer.state.store(RESTORED, Ordering::SeqCst);
        Ok(())
    }
}

/// Borrows the owner thread's existing window and context, never SDL lifetime.
pub struct SdlWorkerRenderContext {
    window: *mut c_void,
    context: *mut c_void,
    sdl: Arc<RenderSdl>,
    state: Arc<AtomicI32>,
    closed: bool,
    render_enabled: bool,
    procedure_leases: Arc<AtomicUsize>,
}

// SAFETY: the worker context is only touched on the adopting thread.
unsafe impl Send for SdlWorkerRenderContext {}

impl SdlWorkerRenderContext {
    /// Adopt a transfer token on a worker thread.
    pub fn adopt(transfer: &SdlRenderContextTransfer, options: &NativeLibraryOptions) -> Result<Self> {
        validate_transfer(transfer.window_id, &transfer.key)?;
        if transfer
            .state
            .compare_exchange(OFFERED, ADOPTED, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(Error::InvalidInput(
                "SDL render context transfer was already consumed".to_string(),
            ));
        }
        // SAFETY: loading maps the image; SDL calls use validated handles.
        unsafe {
            let sdl = RenderSdl::load(options).inspect_err(|_| {
                transfer.state.store(RELEASED, Ordering::SeqCst);
            })?;
            if (sdl.sdl_gl_get_current_context)().is_null() {
                // Expected: the worker must not already hold a context.
            } else {
                transfer.state.store(RELEASED, Ordering::SeqCst);
                return Err(Error::InvalidInput(
                    "worker already has a current SDL context".to_string(),
                ));
            }
            let window = (sdl.sdl_get_window_from_id)(transfer.window_id);
            if window.is_null() {
                transfer.state.store(RELEASED, Ordering::SeqCst);
                return Err(Error::native("SDL_GetWindowFromID", sdl.error()));
            }
            let key = c_string(&transfer.key)?;
            let owner_key = c_string(&format!("{}-owner", transfer.key))?;
            let token = Arc::as_ptr(&transfer.state) as *mut c_void;
            if (sdl.sdl_get_window_data)(window, owner_key.as_ptr()) != token {
                transfer.state.store(RELEASED, Ordering::SeqCst);
                return Err(Error::InvalidInput(
                    "SDL render context ownership does not match the registered transfer".to_string(),
                ));
            }
            let context = (sdl.sdl_get_window_data)(window, key.as_ptr());
            if context.is_null() {
                transfer.state.store(RELEASED, Ordering::SeqCst);
                return Err(Error::native("SDL_GetWindowData", sdl.error()));
            }
            if let Err(error) = sdl.checked((sdl.sdl_gl_make_current)(window, context), "SDL_GL_MakeCurrent adopt") {
                transfer.state.store(RELEASED, Ordering::SeqCst);
                return Err(error);
            }
            Ok(Self {
                window,
                context,
                sdl,
                state: Arc::clone(&transfer.state),
                closed: false,
                render_enabled: true,
                procedure_leases: Arc::new(AtomicUsize::new(0)),
            })
        }
    }

    fn live(&self) -> Result<()> {
        if self.closed || self.state.load(Ordering::SeqCst) != ADOPTED {
            return Err(Error::Closed("SDL worker render context".to_string()));
        }
        Ok(())
    }

    /// Park the context so the owner may use the window list.
    pub fn park(&mut self) -> Result<()> {
        if self.closed {
            return Err(Error::Closed("SDL worker render context".to_string()));
        }
        if self.state.load(Ordering::SeqCst) == PARKED {
            return Ok(());
        }
        if self.state.load(Ordering::SeqCst) != ADOPTED {
            return Err(Error::InvalidInput(
                "SDL worker render context is not adopted".to_string(),
            ));
        }
        // SAFETY: the window handle is live.
        unsafe {
            self.sdl.checked(
                (self.sdl.sdl_gl_make_current)(self.window, std::ptr::null_mut()),
                "SDL_GL_MakeCurrent worker park",
            )?;
        }
        self.state.store(PARKED, Ordering::SeqCst);
        Ok(())
    }

    /// Resume a parked context.
    pub fn resume(&mut self) -> Result<()> {
        if self.closed {
            return Err(Error::Closed("SDL worker render context".to_string()));
        }
        if self.state.load(Ordering::SeqCst) == ADOPTED {
            return Ok(());
        }
        if self
            .state
            .compare_exchange(PARKED, ADOPTED, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(Error::InvalidInput(
                "SDL worker render context is not parked".to_string(),
            ));
        }
        // SAFETY: the window and context handles are live.
        unsafe {
            let context = if self.render_enabled {
                self.context
            } else {
                std::ptr::null_mut()
            };
            if let Err(error) = self.sdl.checked(
                (self.sdl.sdl_gl_make_current)(self.window, context),
                "SDL_GL_MakeCurrent worker resume",
            ) {
                self.state.store(PARKED, Ordering::SeqCst);
                return Err(error);
            }
        }
        Ok(())
    }

    /// Current swap interval.
    pub fn swap_interval(&mut self) -> Result<i32> {
        self.make_current()?;
        // SAFETY: no arguments; result is validated below.
        let value = unsafe { (self.sdl.sdl_gl_get_swap_interval)() };
        if value != -1 && value != 0 && value != 1 {
            return Err(Error::native(
                "SDL_GL_GetSwapInterval",
                format!("unsupported SDL swap interval: {value}"),
            ));
        }
        Ok(value)
    }

    /// Set the swap interval (-1, 0, or 1).
    pub fn set_swap_interval(&mut self, interval: i32) -> Result<()> {
        if interval != -1 && interval != 0 && interval != 1 {
            return Err(Error::OutOfRange("SDL swap interval must be -1, 0, or 1".to_string()));
        }
        self.make_current()?;
        // SAFETY: the interval was validated.
        unsafe {
            self.sdl.checked(
                (self.sdl.sdl_gl_set_swap_interval)(interval),
                "SDL_GL_SetSwapInterval worker",
            )
        }
    }

    /// Release the context back to the owner. Procedure tables must be closed first.
    pub fn release(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        if self.procedure_leases.load(Ordering::SeqCst) != 0 {
            return Err(Error::InvalidInput(
                "close GL procedure tables before releasing the worker context".to_string(),
            ));
        }
        // SAFETY: the window handle is live.
        unsafe {
            self.sdl.checked(
                (self.sdl.sdl_gl_make_current)(self.window, std::ptr::null_mut()),
                "SDL_GL_MakeCurrent worker release",
            )?;
        }
        self.closed = true;
        self.state.store(RELEASED, Ordering::SeqCst);
        Ok(())
    }

    /// Whether the context is released.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

impl SdlRenderContext for SdlWorkerRenderContext {
    fn drawable_size(&mut self) -> Result<(u32, u32)> {
        self.make_current()?;
        let mut width = 0i32;
        let mut height = 0i32;
        // SAFETY: out-pointers describe live integers.
        unsafe {
            (self.sdl.sdl_gl_get_drawable_size)(self.window, &mut width, &mut height);
        }
        if width <= 0 || height <= 0 {
            return Err(Error::native(
                "SDL_GL_GetDrawableSize",
                "SDL returned invalid drawable dimensions".to_string(),
            ));
        }
        Ok((width as u32, height as u32))
    }

    fn rendering_enabled(&self) -> bool {
        self.render_enabled
    }

    fn set_rendering_enabled(&mut self, enabled: bool) -> Result<()> {
        self.live()?;
        // SAFETY: the window and context handles are live.
        unsafe {
            let context = if enabled { self.context } else { std::ptr::null_mut() };
            self.sdl.checked(
                (self.sdl.sdl_gl_make_current)(self.window, context),
                "SDL_GL_MakeCurrent worker diagnostic",
            )?;
        }
        self.render_enabled = enabled;
        Ok(())
    }

    fn make_current(&mut self) -> Result<()> {
        self.live()?;
        // SAFETY: the window and context handles are live.
        unsafe {
            let context = if self.render_enabled {
                self.context
            } else {
                std::ptr::null_mut()
            };
            self.sdl.checked(
                (self.sdl.sdl_gl_make_current)(self.window, context),
                "SDL_GL_MakeCurrent worker",
            )
        }
    }

    fn get_gl_proc_address(&mut self, name: &str) -> Result<*mut c_void> {
        if name.is_empty() || name.contains('\0') {
            return Err(Error::InvalidInput("invalid GL procedure name".to_string()));
        }
        self.make_current()?;
        let symbol = c_string(name)?;
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

    fn retain_procedures(&mut self) -> Result<ProcedureGuard> {
        self.make_current()?;
        Ok(ProcedureGuard::new(Arc::clone(&self.procedure_leases)))
    }

    fn swap(&mut self) -> Result<()> {
        self.make_current()?;
        // SAFETY: the window and context handles are live.
        unsafe {
            // SDL requires a current window to swap; Mac flushBuffer did not.
            if !self.render_enabled {
                self.sdl.checked(
                    (self.sdl.sdl_gl_make_current)(self.window, self.context),
                    "SDL_GL_MakeCurrent worker swap",
                )?;
            }
            (self.sdl.sdl_gl_swap_window)(self.window);
            if !self.render_enabled {
                self.sdl.checked(
                    (self.sdl.sdl_gl_make_current)(self.window, std::ptr::null_mut()),
                    "SDL_GL_MakeCurrent worker restore diagnostic",
                )?;
            }
        }
        Ok(())
    }
}

impl Drop for SdlWorkerRenderContext {
    fn drop(&mut self) {
        self.release().ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_key_validation() {
        assert!(validate_transfer(7, "quake-render-12345678-1234-1234-1234-1234567890ab").is_ok());
        assert!(validate_transfer(0, "quake-render-12345678-1234-1234-1234-1234567890ab").is_err());
        assert!(validate_transfer(7, "quake-render-short").is_err());
        assert!(validate_transfer(7, "other-12345678-1234-1234-1234-1234567890ab").is_err());
        assert!(validate_transfer(7, "quake-render-zzzzzzzz-1234-1234-1234-1234567890ab").is_err());
        let key = random_transfer_key();
        assert!(is_transfer_key(&key), "{key}");
        assert!(validate_transfer(1, &key).is_ok());
    }

    #[test]
    fn adopt_rejects_consumed_transfer_without_library() {
        let transfer = SdlRenderContextTransfer {
            window_id: 3,
            key: "quake-render-12345678-1234-1234-1234-1234567890ab".to_string(),
            state: Arc::new(AtomicI32::new(ADOPTED)),
        };
        let mut environment = std::collections::HashMap::new();
        environment.insert(
            "QUAKE_SDL2_LIBRARY".to_string(),
            "/nonexistent-qa-platform/libSDL2.so".to_string(),
        );
        let options = NativeLibraryOptions {
            environment: Some(environment),
            ..NativeLibraryOptions::default()
        };
        // The consumed-transfer check runs before any library load.
        let Err(error) = SdlWorkerRenderContext::adopt(&transfer, &options) else {
            panic!("expected failure")
        };
        assert!(error.to_string().contains("already consumed"), "{error}");
    }

    #[test]
    fn adopt_without_library_names_sdl2() {
        let transfer = SdlRenderContextTransfer {
            window_id: 3,
            key: "quake-render-12345678-1234-1234-1234-1234567890ab".to_string(),
            state: Arc::new(AtomicI32::new(OFFERED)),
        };
        let mut environment = std::collections::HashMap::new();
        environment.insert(
            "QUAKE_SDL2_LIBRARY".to_string(),
            "/nonexistent-qa-platform/libSDL2.so".to_string(),
        );
        let options = NativeLibraryOptions {
            environment: Some(environment),
            ..NativeLibraryOptions::default()
        };
        let Err(error) = SdlWorkerRenderContext::adopt(&transfer, &options) else {
            panic!("expected failure")
        };
        assert!(error.is_unavailable(), "{error}");
        assert!(error.to_string().contains("sdl2"), "{error}");
        assert_eq!(transfer.state.load(Ordering::SeqCst), RELEASED);
    }

    #[test]
    fn procedure_guard_counts_leases() {
        let counter = Arc::new(AtomicUsize::new(0));
        let guard = ProcedureGuard::new(Arc::clone(&counter));
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        guard.release();
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        let guard = ProcedureGuard::new(Arc::clone(&counter));
        drop(guard);
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }
}
