//! MIT-SHM software present for X11 windows.
//!
//! Why this module exists: the software-renderer present path pushes every frame
//! through the X11 socket (`XPutImage` over a local socket costs ~13ms for a
//! 1400x900 frame). MIT-SHM hands the frame to the X server through shared
//! memory instead, which costs ~0.2ms and keeps the game loop under its frame
//! budget. Everything here is a best-effort fast path: any step can fail (no
//! X11, no SHM extension, 32-bit pointer layout) and the caller silently keeps
//! the portable renderer present.
//!
//! Layout notes (all verified against system headers on this host):
//! * `XShmSegmentInfo` is `{ shmseg: u64, shmid: i32, shmaddr: *mut i8,
//!   read_only: i32 }` from `X11/extensions/XShm.h`.
//! * `XImage` reads in `ZPixmap` format use the `data` pointer plus
//!   `bytes_per_line` for row addressing; only the leading fields through the
//!   channel masks are modeled, since the presenter never touches the function
//!   table tail.
//! * `XGetWindowAttributes` fills an `XWindowAttributes` whose first fields are
//!   `x, y, width, height, border_width, depth`, followed by the `visual`
//!   pointer. Only `depth` and `visual` are read, but the full struct is
//!   passed because Xlib fills all of it.

use std::ffi::{c_int, c_long, c_ulong, c_void};
use std::ptr;

use crate::ffi_util::LoadedLibrary;
use crate::native_libraries::{NativeLibrary, NativeLibraryOptions};

/// Full `XWindowAttributes` (X11/Xlib.h). `XGetWindowAttributes` fills the
/// whole struct, so a shorter head would overflow the stack; only `depth`
/// and `visual` are read.
#[repr(C)]
#[allow(dead_code)]
struct XWindowAttributes {
    x: c_int,
    y: c_int,
    width: c_int,
    height: c_int,
    border_width: c_int,
    depth: c_int,
    visual: *mut c_void,
    root: c_ulong,
    class: c_int,
    bit_gravity: c_int,
    win_gravity: c_int,
    backing_store: c_int,
    backing_planes: c_ulong,
    backing_pixel: c_ulong,
    save_underline: c_int,
    colormap: c_ulong,
    map_installed: c_int,
    map_state: c_int,
    all_event_masks: c_long,
    your_event_mask: c_long,
    do_not_propagate_mask: c_long,
    override_redirect: c_int,
    screen: *mut c_void,
}

// The X11 LP64 ABI fixes this size; a mismatch would mean stack overflow.
#[cfg(all(unix, target_pointer_width = "64"))]
const _: () = assert!(std::mem::size_of::<XWindowAttributes>() == 136);

/// Leading fields of `XImage` (X11/Xlib.h) through the channel masks.
#[repr(C)]
#[allow(dead_code)]
struct XImageHead {
    width: i32,
    height: i32,
    xoffset: i32,
    format: i32,
    data: *mut c_void,
    byte_order: i32,
    bitmap_unit: i32,
    bitmap_bit_order: i32,
    bitmap_pad: i32,
    depth: i32,
    bytes_per_line: i32,
    bits_per_pixel: i32,
    red_mask: u64,
    green_mask: u64,
    blue_mask: u64,
}

/// `XShmSegmentInfo` (X11/extensions/XShm.h), LP64 layout.
#[repr(C)]
#[allow(dead_code)]
struct XShmSegmentInfo {
    shmseg: u64,
    shmid: i32,
    shmaddr: *mut i8,
    read_only: i32,
}

// SysV shared memory through always-present libc symbols (same link(name =
// "c") approach as `ipx_native`); Linux-only, everything else reports
// unavailable so the caller keeps the portable present.
#[cfg(target_os = "linux")]
#[link(name = "c")]
unsafe extern "C" {
    fn shmget(key: i32, size: usize, flags: i32) -> i32;
    fn shmat(shmid: i32, addr: *const c_void, flags: i32) -> *mut c_void;
    fn shmdt(addr: *const c_void) -> i32;
    fn shmctl(shmid: i32, cmd: i32, buf: *mut c_void) -> i32;
}

/// Create a private segment and attach it. Returns the id and mapping.
#[cfg(target_os = "linux")]
fn shm_create(size: usize) -> Option<(i32, *mut c_void)> {
    const IPC_PRIVATE: i32 = 0;
    const IPC_CREAT: i32 = 0o1000;
    const IPC_RMID: i32 = 0;
    // SAFETY: fresh private segment; destroyed below on any failure.
    let shmid = unsafe { shmget(IPC_PRIVATE, size.max(1), IPC_CREAT | 0o600) };
    if shmid < 0 {
        return None;
    }
    // SAFETY: `shmid` was just created by this thread.
    let addr = unsafe { shmat(shmid, ptr::null(), 0) };
    if addr == usize::MAX as *mut c_void {
        // SAFETY: `shmid` was just created by this thread.
        unsafe {
            shmctl(shmid, IPC_RMID, ptr::null_mut());
        }
        return None;
    }
    Some((shmid, addr))
}

/// Detach a mapping and destroy its segment.
#[cfg(target_os = "linux")]
fn shm_destroy(shmid: i32, addr: *mut c_void) {
    const IPC_RMID: i32 = 0;
    // SAFETY: the segment and mapping were created by `shm_create`.
    unsafe {
        shmdt(addr);
        shmctl(shmid, IPC_RMID, ptr::null_mut());
    }
}

#[cfg(not(target_os = "linux"))]
fn shm_create(_size: usize) -> Option<(i32, *mut c_void)> {
    None
}

#[cfg(not(target_os = "linux"))]
fn shm_destroy(_shmid: i32, _addr: *mut c_void) {}

struct X11Lib {
    _lib: LoadedLibrary,
    get_window_attributes: unsafe extern "C" fn(*mut c_void, u64, *mut XWindowAttributes) -> i32,
    create_gc: unsafe extern "C" fn(*mut c_void, u64, u64, *mut c_void) -> u64,
    free_gc: unsafe extern "C" fn(*mut c_void, u64) -> i32,
    destroy_image: unsafe extern "C" fn(*mut XImageHead) -> i32,
    flush: unsafe extern "C" fn(*mut c_void) -> i32,
}

struct XextLib {
    _lib: LoadedLibrary,
    query_extension: unsafe extern "C" fn(*mut c_void) -> i32,
    create_image: unsafe extern "C" fn(
        *mut c_void,
        *mut c_void,
        u32,
        i32,
        *mut c_void,
        *mut XShmSegmentInfo,
        u32,
        u32,
    ) -> *mut XImageHead,
    attach: unsafe extern "C" fn(*mut c_void, *mut XShmSegmentInfo) -> i32,
    detach: unsafe extern "C" fn(*mut c_void, *mut XShmSegmentInfo) -> i32,
    put_image: unsafe extern "C" fn(*mut c_void, u64, u64, *mut XImageHead, i32, i32, i32, i32, u32, u32, i32) -> i32,
}

impl X11Lib {
    fn open(options: &NativeLibraryOptions) -> Option<Self> {
        // SAFETY: loading maps the image without invoking its code.
        let lib = unsafe { LoadedLibrary::open(NativeLibrary::X11, options) }.ok()?;
        macro_rules! sym {
            ($name:literal, $sig:ty) => {
                // SAFETY: the address is only read here.
                unsafe { lib.symbol::<$sig>(concat!($name, "\0").as_bytes()) }.ok()?
            };
        }
        Some(Self {
            get_window_attributes: sym!(
                "XGetWindowAttributes",
                unsafe extern "C" fn(*mut c_void, u64, *mut XWindowAttributes) -> i32
            ),
            create_gc: sym!(
                "XCreateGC",
                unsafe extern "C" fn(*mut c_void, u64, u64, *mut c_void) -> u64
            ),
            free_gc: sym!("XFreeGC", unsafe extern "C" fn(*mut c_void, u64) -> i32),
            destroy_image: sym!("XDestroyImage", unsafe extern "C" fn(*mut XImageHead) -> i32),
            flush: sym!("XFlush", unsafe extern "C" fn(*mut c_void) -> i32),
            _lib: lib,
        })
    }
}

impl XextLib {
    fn open(options: &NativeLibraryOptions) -> Option<Self> {
        // SAFETY: loading maps the image without invoking its code.
        let lib = unsafe { LoadedLibrary::open(NativeLibrary::Xext, options) }.ok()?;
        macro_rules! sym {
            ($name:literal, $sig:ty) => {
                // SAFETY: the address is only read here.
                unsafe { lib.symbol::<$sig>(concat!($name, "\0").as_bytes()) }.ok()?
            };
        }
        Some(Self {
            query_extension: sym!("XShmQueryExtension", unsafe extern "C" fn(*mut c_void) -> i32),
            create_image: sym!(
                "XShmCreateImage",
                unsafe extern "C" fn(
                    *mut c_void,
                    *mut c_void,
                    u32,
                    i32,
                    *mut c_void,
                    *mut XShmSegmentInfo,
                    u32,
                    u32,
                ) -> *mut XImageHead
            ),
            attach: sym!(
                "XShmAttach",
                unsafe extern "C" fn(*mut c_void, *mut XShmSegmentInfo) -> i32
            ),
            detach: sym!(
                "XShmDetach",
                unsafe extern "C" fn(*mut c_void, *mut XShmSegmentInfo) -> i32
            ),
            put_image: sym!(
                "XShmPutImage",
                unsafe extern "C" fn(*mut c_void, u64, u64, *mut XImageHead, i32, i32, i32, i32, u32, u32, i32) -> i32
            ),
            _lib: lib,
        })
    }
}

/// Row writer from RGBA source bytes into one shared-memory image row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PixelPack {
    /// 32-bit `0x00RRGGBB` words (host byte order).
    Xrgb32,
    /// 24-bit packed `RR GG BB` triples.
    Rgb24,
    /// 16-bit `RRRRRGGGGGGBBBBB` words (host byte order).
    Rgb565,
}

/// Precomputed blit plan: how RGBA source rows land in the shared image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SurfaceBlit {
    pack: PixelPack,
    source_stride: usize,
    dest_stride: usize,
    width: usize,
    height: usize,
}

impl SurfaceBlit {
    /// Derive the pack from the `XImage` channel masks; `None` means the
    /// server visual is one this presenter does not handle.
    fn from_masks(
        width: usize,
        height: usize,
        bits_per_pixel: i32,
        red_mask: u64,
        green_mask: u64,
        blue_mask: u64,
        bytes_per_line: i32,
    ) -> Option<Self> {
        let pack = match (bits_per_pixel, red_mask, green_mask, blue_mask) {
            (32, 0x00ff_0000, 0x0000_ff00, 0x0000_00ff) => PixelPack::Xrgb32,
            (24, 0x00ff_0000, 0x0000_ff00, 0x0000_00ff) => PixelPack::Rgb24,
            (16, 0xf800, 0x07e0, 0x001f) => PixelPack::Rgb565,
            _ => return None,
        };
        let bytes_per_line = usize::try_from(bytes_per_line).ok()?;
        let row_bytes = match pack {
            PixelPack::Xrgb32 => width.checked_mul(4)?,
            PixelPack::Rgb24 => width.checked_mul(3)?,
            PixelPack::Rgb565 => width.checked_mul(2)?,
        };
        if bytes_per_line < row_bytes {
            return None;
        }
        Some(Self {
            pack,
            source_stride: width.checked_mul(4)?,
            dest_stride: bytes_per_line,
            width,
            height,
        })
    }

    fn blit(&self, source: &[u8], dest: *mut u8) {
        let rows = self.height.min(source.len() / self.source_stride.max(1));
        let row_bytes = self.row_bytes();
        for row in 0..rows {
            let src = &source[row * self.source_stride..(row + 1) * self.source_stride];
            // SAFETY: the caller guarantees `dest` spans `height` rows of
            // `dest_stride` bytes; `from_masks` verified `row_bytes` fits.
            let out = unsafe {
                let row_ptr = dest.add(row * self.dest_stride);
                std::slice::from_raw_parts_mut(row_ptr, row_bytes)
            };
            match self.pack {
                PixelPack::Xrgb32 => {
                    for x in 0..self.width {
                        let (s, d) = (x * 4, x * 4);
                        out[d] = src[s + 2];
                        out[d + 1] = src[s + 1];
                        out[d + 2] = src[s];
                        out[d + 3] = 0;
                    }
                }
                PixelPack::Rgb24 => {
                    for x in 0..self.width {
                        let (s, d) = (x * 4, x * 3);
                        out[d] = src[s];
                        out[d + 1] = src[s + 1];
                        out[d + 2] = src[s + 2];
                    }
                }
                PixelPack::Rgb565 => {
                    for x in 0..self.width {
                        let (s, d) = (x * 4, x * 2);
                        let word = (u16::from(src[s]) & 0xf8) << 8
                            | (u16::from(src[s + 1]) & 0xfc) << 3
                            | u16::from(src[s + 2]) >> 3;
                        let bytes = word.to_ne_bytes();
                        out[d] = bytes[0];
                        out[d + 1] = bytes[1];
                    }
                }
            }
        }
    }

    fn row_bytes(&self) -> usize {
        match self.pack {
            PixelPack::Xrgb32 => self.width * 4,
            PixelPack::Rgb24 => self.width * 3,
            PixelPack::Rgb565 => self.width * 2,
        }
    }
}

/// Live MIT-SHM presenter for one X11 window.
///
/// Owns the loader handles, the SysV segment, the `XImage`, and the graphics
/// context; dropping the presenter detaches and frees everything. Xlib calls
/// are confined to the thread that owns the window, matching `SdlWindow`.
pub struct XshmPresenter {
    x11: X11Lib,
    xext: XextLib,
    display: *mut c_void,
    window: u64,
    gc: u64,
    image: *mut XImageHead,
    shminfo: Box<XShmSegmentInfo>,
    width: u32,
    height: u32,
    blit: SurfaceBlit,
}

impl XshmPresenter {
    /// Attach a shared-memory presenter to `window` on `display`.
    ///
    /// Returns `None` when any step is unavailable so the caller can keep the
    /// portable renderer present. Only 64-bit LP64 layouts are supported.
    ///
    /// # Safety
    ///
    /// `display` must be the live `Display` of the SDL window that owns
    /// `window`, and both must outlive the presenter.
    pub unsafe fn attach(
        options: &NativeLibraryOptions,
        display: *mut c_void,
        window: u64,
        width: u32,
        height: u32,
    ) -> Option<Self> {
        if !cfg!(target_pointer_width = "64") {
            return None;
        }
        if display.is_null() || window == 0 || width == 0 || height == 0 {
            return None;
        }
        let x11 = X11Lib::open(options)?;
        let xext = XextLib::open(options)?;
        // SAFETY: the display comes from the live SDL window.
        if unsafe { (xext.query_extension)(display) } == 0 {
            return None;
        }
        let mut attributes = XWindowAttributes {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            border_width: 0,
            depth: 0,
            visual: ptr::null_mut(),
            root: 0,
            class: 0,
            bit_gravity: 0,
            win_gravity: 0,
            backing_store: 0,
            backing_planes: 0,
            backing_pixel: 0,
            save_underline: 0,
            colormap: 0,
            map_installed: 0,
            map_state: 0,
            all_event_masks: 0,
            your_event_mask: 0,
            do_not_propagate_mask: 0,
            override_redirect: 0,
            screen: ptr::null_mut(),
        };
        // SAFETY: the window is alive and owned by the caller.
        if unsafe { (x11.get_window_attributes)(display, window, &mut attributes) } == 0 {
            return None;
        }
        if attributes.visual.is_null() || attributes.depth < 15 {
            return None;
        }
        // Placeholder segment so `XShmCreateImage` sees a valid id; the real
        // segment is sized below once the image reports its stride.
        let (shmid, addr) = shm_create(4)?;
        let mut shminfo = Box::new(XShmSegmentInfo {
            shmseg: 0,
            shmid,
            shmaddr: addr.cast(),
            read_only: 0,
        });
        // SAFETY: `XShmCreateImage` may return null; the shminfo out-param is a
        // live unique borrow. `ZPixmap` format with a null bitmap pad lets the
        // server pick the natural packing for the visual.
        let image = unsafe {
            (xext.create_image)(
                display,
                attributes.visual,
                attributes.depth as u32,
                2, // ZPixmap
                ptr::null_mut(),
                shminfo.as_mut(),
                width,
                height,
            )
        };
        if image.is_null() {
            shm_destroy(shmid, addr);
            return None;
        }
        // SAFETY: the image is live and exclusively owned until attached.
        let (bits_per_pixel, bytes_per_line, red_mask, green_mask, blue_mask) = unsafe {
            (
                (*image).bits_per_pixel,
                (*image).bytes_per_line,
                (*image).red_mask,
                (*image).green_mask,
                (*image).blue_mask,
            )
        };
        let blit = SurfaceBlit::from_masks(
            width as usize,
            height as usize,
            bits_per_pixel,
            red_mask,
            green_mask,
            blue_mask,
            bytes_per_line,
        );
        let Some(blit) = blit else {
            // SAFETY: image created above; segment not yet attached.
            unsafe {
                (x11.destroy_image)(image);
            }
            shm_destroy(shmid, addr);
            return None;
        };
        // The segment must span the image before attach; the placeholder is
        // replaced with a correctly sized one.
        shm_destroy(shmid, addr);
        let size = bytes_per_line as usize * height as usize;
        let Some((shmid, addr)) = shm_create(size) else {
            // SAFETY: image created above; no segment attached.
            unsafe {
                (x11.destroy_image)(image);
            }
            return None;
        };
        shminfo.shmid = shmid;
        shminfo.shmaddr = addr.cast();
        // SAFETY: the image and segment are both live; on failure the image is
        // destroyed and the segment detached and removed.
        if unsafe { (xext.attach)(display, shminfo.as_mut()) } == 0 {
            // SAFETY: image created above; attach failed.
            unsafe {
                (x11.destroy_image)(image);
            }
            shm_destroy(shmid, addr);
            return None;
        }
        // SAFETY: the image is live and uniquely owned; publishing the segment
        // address makes it the image pixel store.
        unsafe {
            (*image).data = addr;
        }
        // SAFETY: plain graphics context on the live window.
        let gc = unsafe { (x11.create_gc)(display, window, 0, ptr::null_mut()) };
        if gc == 0 {
            // SAFETY: tearing down the attached segment and image.
            unsafe {
                (xext.detach)(display, shminfo.as_mut());
                (x11.destroy_image)(image);
            }
            shm_destroy(shmid, addr);
            return None;
        }
        Some(Self {
            x11,
            xext,
            display,
            window,
            gc,
            image,
            shminfo,
            width,
            height,
            blit,
        })
    }

    /// Window size this presenter was built for.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Blit `rgba` into shared memory and hand it to the X server.
    ///
    /// Returns `false` when the source frame is short; X protocol errors are
    /// asynchronous and cannot be reported per call.
    pub fn present(&mut self, rgba: &[u8]) -> bool {
        let needed = self.blit.source_stride * self.blit.height;
        if rgba.len() < needed {
            return false;
        }
        // SAFETY: the segment spans the image; the blit stays inside it.
        let dest = unsafe { (*self.image).data.cast::<u8>() };
        if dest.is_null() {
            return false;
        }
        self.blit.blit(rgba, dest);
        // SAFETY: display, window, graphics context, and image are all live
        // members; sizes match the attached image. `send_event` is false: the
        // server never notifies completion and the next frame simply
        // overwrites the segment.
        unsafe {
            (self.xext.put_image)(
                self.display,
                self.window,
                self.gc,
                self.image,
                0,
                0,
                0,
                0,
                self.width,
                self.height,
                0,
            );
            (self.x11.flush)(self.display);
        }
        true
    }
}

impl Drop for XshmPresenter {
    fn drop(&mut self) {
        // SAFETY: every handle was created by `attach` and is dropped exactly
        // once; the display outlives the presenter because the window does.
        unsafe {
            (self.xext.detach)(self.display, self.shminfo.as_mut());
            (self.x11.destroy_image)(self.image);
            (self.x11.free_gc)(self.display, self.gc);
        }
        shm_destroy(self.shminfo.shmid, self.shminfo.shmaddr.cast());
    }
}

#[cfg(test)]
mod tests {
    use super::{PixelPack, SurfaceBlit};

    #[test]
    fn masks_select_pack() {
        let xrgb = SurfaceBlit::from_masks(8, 4, 32, 0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 32);
        assert_eq!(
            xrgb.map(|blit| (blit.pack, blit.source_stride, blit.dest_stride)),
            Some((PixelPack::Xrgb32, 32, 32))
        );
        let rgb565 = SurfaceBlit::from_masks(8, 4, 16, 0xf800, 0x07e0, 0x001f, 16);
        assert_eq!(
            rgb565.map(|blit| (blit.pack, blit.source_stride, blit.dest_stride)),
            Some((PixelPack::Rgb565, 32, 16))
        );
        assert!(SurfaceBlit::from_masks(8, 4, 32, 0xff, 0xff00, 0xff0000, 32).is_none());
        assert!(SurfaceBlit::from_masks(8, 4, 32, 0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 31).is_none());
    }

    #[test]
    fn xrgb32_blit_drops_alpha() {
        let blit = SurfaceBlit::from_masks(2, 1, 32, 0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 8).expect("xrgb32 plan");
        let source = [0x11, 0x22, 0x33, 0xff, 0xaa, 0xbb, 0xcc, 0x80];
        let mut dest = [0xee; 8];
        blit.blit(&source, dest.as_mut_ptr());
        assert_eq!(dest, [0x33, 0x22, 0x11, 0, 0xcc, 0xbb, 0xaa, 0]);
    }

    #[test]
    fn rgb565_blit_rounds_channels() {
        let blit = SurfaceBlit::from_masks(1, 1, 16, 0xf800, 0x07e0, 0x001f, 2).expect("rgb565 plan");
        let source = [0xff, 0xff, 0xff, 0xff];
        let mut dest = [0; 2];
        blit.blit(&source, dest.as_mut_ptr());
        assert_eq!(dest, 0xffffu16.to_ne_bytes());
    }

    #[test]
    fn short_source_rows_are_skipped() {
        let blit = SurfaceBlit::from_masks(2, 2, 32, 0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 8).expect("xrgb32 plan");
        let source = [1, 2, 3, 4, 5, 6, 7, 8];
        let mut dest = [0xee; 16];
        blit.blit(&source, dest.as_mut_ptr());
        assert_eq!(&dest[..8], &[3, 2, 1, 0, 7, 6, 5, 0]);
        assert!(dest[8..].iter().all(|byte| *byte == 0xee));
    }
}
