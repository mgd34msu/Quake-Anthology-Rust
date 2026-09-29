//! FreeType outline and bitmap glyph rasterization.
//!
//! Port of donor `src/platform/freetype.ts`. Outline raster metrics follow id
//! Software's `code/renderer/tr_font.c`; only the qualified little-endian
//! LP64/LLP64 ABIs from [`freetype_layout`](crate::freetype_layout) are used.
//! Memory copies use core pointer operations instead of libc `memcpy`.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::ffi_util::LoadedLibrary;
use crate::freetype_layout::{
    free_type_bitmap_length, free_type_metric, host_free_type_layout, normalize_free_type_bitmap_rows, FreeTypeLayout,
};
use crate::native_libraries::{NativeLibrary, NativeLibraryOptions};

/// Typed FreeType failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreeTypeError {
    /// Operation name, e.g. `"FT_Load_Glyph"`.
    pub operation: String,
    /// FreeType error code.
    pub code: i32,
}

impl FreeTypeError {
    fn new(operation: impl Into<String>, code: i32) -> Self {
        Self {
            operation: operation.into(),
            code,
        }
    }
}

impl std::fmt::Display for FreeTypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} failed with FreeType error {}", self.operation, self.code)
    }
}

impl std::error::Error for FreeTypeError {}

impl From<FreeTypeError> for Error {
    fn from(error: FreeTypeError) -> Self {
        Self::Coded {
            operation: error.operation,
            code: i64::from(error.code),
        }
    }
}

/// Opaque face handle. Identity is checked against the owning library; native
/// addresses stay private.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FreeTypeFace {
    id: u64,
}

struct FaceRecord {
    address: u64,
    /// Owned font bytes; live until the face is released or the library closes.
    #[allow(dead_code)]
    bytes: Vec<u8>,
}

/// Q3 outline glyph bitmap with padded pitch and source top/xSkip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontGlyphBitmap {
    /// Width in pixels.
    pub width: i32,
    /// Height in pixels.
    pub height: i32,
    /// Padded row stride (multiple of 4).
    pub pitch: i32,
    /// Source top (`bearingY / 64 + 1`).
    pub top: i32,
    /// Source bottom in 26.6 units.
    pub bottom: i32,
    /// Source horizontal advance (`advance / 64 + 1`).
    pub x_skip: i32,
    /// Grayscale pixels, top to bottom.
    pub pixels: Vec<u8>,
}

/// Native bitmap glyph with owned top-to-bottom pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeFontGlyphBitmap {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Positive row stride of the owned pixels.
    pub pitch: i32,
    /// Native (possibly negative) pitch.
    pub native_pitch: i32,
    /// FreeType pixel mode.
    pub pixel_mode: u8,
    /// Number of grays.
    pub num_grays: u16,
    /// Left bearing.
    pub left: i32,
    /// Top bearing.
    pub top: i32,
    /// Horizontal advance in 26.6 units.
    pub advance_x26: i32,
    /// Vertical advance in 26.6 units.
    pub advance_y26: i32,
    /// Owned pixels, top to bottom.
    pub pixels: Vec<u8>,
}

/// Rounded 26.6 outline box plus validated pixel dimensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutlineBox {
    /// Rounded left in 26.6 units.
    pub left: i32,
    /// Rounded right in 26.6 units.
    pub right: i32,
    /// Rounded top in 26.6 units.
    pub top: i32,
    /// Rounded bottom in 26.6 units.
    pub bottom: i32,
    /// Width in pixels.
    pub width: i32,
    /// Height in pixels.
    pub height: i32,
    /// Padded pitch (multiple of 4).
    pub pitch: i32,
}

/// Validate a FreeType pixel size (positive int32 26.6 pixels).
pub fn validate_glyph_size(size: i32) -> Result<()> {
    if size <= 0 || size > 0x01ff_ffff {
        return Err(Error::OutOfRange(
            "FreeType size must be a positive int32 26.6 pixel size".to_string(),
        ));
    }
    Ok(())
}

/// Validate a Unicode scalar value for glyph lookup.
pub fn validate_code_point(code: u32) -> Result<()> {
    if code > 0x10_ffff || (0xd800..=0xdfff).contains(&code) {
        return Err(Error::OutOfRange(
            "FreeType code point must be a Unicode scalar value".to_string(),
        ));
    }
    Ok(())
}

/// Round outline metrics to the 26.6 pixel box, rejecting source-int32 overflow.
pub fn outline_box(width26: i32, height26: i32, bearing_x: i32, bearing_y: i32) -> Result<OutlineBox> {
    if width26 < 0 || height26 < 0 {
        return Err(Error::OutOfRange("FreeType outline has negative extents".to_string()));
    }
    let floor64 = |value: i64| value.div_euclid(64) * 64;
    let ceil64 = |value: i64| value.div_euclid(64) * 64 + i64::from(value.rem_euclid(64) != 0) * 64;
    let left = floor64(i64::from(bearing_x));
    let right = ceil64(i64::from(bearing_x) + i64::from(width26));
    let top = ceil64(i64::from(bearing_y));
    let bottom = floor64(i64::from(bearing_y) - i64::from(height26));
    for negated in [-left, -bottom] {
        if negated < i64::from(i32::MIN) || negated > i64::from(i32::MAX) {
            return Err(Error::OutOfRange(
                "FreeType outline translation exceeds source int32".to_string(),
            ));
        }
    }
    let width = (right - left) / 64;
    let height = (top - bottom) / 64;
    if width < 0 || height < 0 {
        return Err(Error::OutOfRange("FreeType outline has negative extents".to_string()));
    }
    let (left, right, top, bottom, width, height) = (
        left as i32,
        right as i32,
        top as i32,
        bottom as i32,
        width as i32,
        height as i32,
    );
    Ok(OutlineBox {
        left,
        right,
        top,
        bottom,
        width,
        height,
        pitch: width.div_euclid(4) * 4 + if width.rem_euclid(4) == 0 { 0 } else { 4 },
    })
}

struct FreeTypeLib {
    _lib: LoadedLibrary,
    layout: FreeTypeLayout,
    ft_init_free_type: *mut c_void,
    ft_done_free_type: *mut c_void,
    ft_new_memory_face: *mut c_void,
    ft_done_face: *mut c_void,
    ft_set_char_size: *mut c_void,
    ft_select_charmap: *mut c_void,
    ft_get_char_index: *mut c_void,
    ft_load_glyph: *mut c_void,
    ft_render_glyph: *mut c_void,
    ft_outline_translate: *mut c_void,
    ft_outline_get_bitmap: *mut c_void,
}

// SAFETY: raw addresses are only called with the signatures below.
unsafe impl Send for FreeTypeLib {}
unsafe impl Sync for FreeTypeLib {}

impl FreeTypeLib {
    /// # Safety
    ///
    /// Resolved symbols are only invoked through the wrappers below, which
    /// select the LP64/LLP64 signature from `layout`.
    unsafe fn load(layout: FreeTypeLayout, options: &NativeLibraryOptions) -> Result<Self> {
        // SAFETY: loading maps the image without invoking its code.
        let lib = unsafe { LoadedLibrary::open(NativeLibrary::FreeType, options)? };
        macro_rules! sym {
            ($name:literal) => {
                // SAFETY: the address is only read here.
                unsafe { lib.symbol::<*mut c_void>(concat!($name, "\0").as_bytes())? }
            };
        }
        Ok(Self {
            ft_init_free_type: sym!("FT_Init_FreeType"),
            ft_done_free_type: sym!("FT_Done_FreeType"),
            ft_new_memory_face: sym!("FT_New_Memory_Face"),
            ft_done_face: sym!("FT_Done_Face"),
            ft_set_char_size: sym!("FT_Set_Char_Size"),
            ft_select_charmap: sym!("FT_Select_Charmap"),
            ft_get_char_index: sym!("FT_Get_Char_Index"),
            ft_load_glyph: sym!("FT_Load_Glyph"),
            ft_render_glyph: sym!("FT_Render_Glyph"),
            ft_outline_translate: sym!("FT_Outline_Translate"),
            ft_outline_get_bitmap: sym!("FT_Outline_Get_Bitmap"),
            layout,
            _lib: lib,
        })
    }

    /// # Safety
    ///
    /// `out` must be a live `FT_Library*` out-pointer.
    unsafe fn init(&self, out: *mut u64) -> i32 {
        let f: unsafe extern "C" fn(*mut u64) -> i32 =
            // SAFETY: the signature matches FreeType on every qualified ABI.
            unsafe { std::mem::transmute(self.ft_init_free_type) };
        // SAFETY: caller guarantees a live out-pointer.
        unsafe { f(out) }
    }

    /// # Safety
    ///
    /// `library` must be a live `FT_Library`.
    unsafe fn done(&self, library: u64) -> i32 {
        let f: unsafe extern "C" fn(u64) -> i32 =
            // SAFETY: the signature matches FreeType on every qualified ABI.
            unsafe { std::mem::transmute(self.ft_done_free_type) };
        // SAFETY: caller guarantees a live library.
        unsafe { f(library) }
    }

    /// # Safety
    ///
    /// `bytes`/`out` must be live; `bytes` must outlive the created face.
    unsafe fn new_memory_face(
        &self,
        library: u64,
        bytes: *const u8,
        length: usize,
        face_index: i64,
        out: *mut u64,
    ) -> i32 {
        // SAFETY: the signature branch matches the qualified ABI.
        unsafe {
            if self.layout.long_bytes == 4 {
                let f: unsafe extern "C" fn(u64, *const u8, i32, i32, *mut u64) -> i32 =
                    std::mem::transmute(self.ft_new_memory_face);
                f(library, bytes, length as i32, face_index as i32, out)
            } else {
                let f: unsafe extern "C" fn(u64, *const u8, i64, i64, *mut u64) -> i32 =
                    std::mem::transmute(self.ft_new_memory_face);
                f(library, bytes, length as i64, face_index, out)
            }
        }
    }

    /// # Safety
    ///
    /// `face` must be a live `FT_Face`.
    unsafe fn done_face(&self, face: u64) -> i32 {
        let f: unsafe extern "C" fn(u64) -> i32 =
            // SAFETY: the signature matches FreeType on every qualified ABI.
            unsafe { std::mem::transmute(self.ft_done_face) };
        // SAFETY: caller guarantees a live face.
        unsafe { f(face) }
    }

    /// # Safety
    ///
    /// `face` must be a live `FT_Face`.
    unsafe fn set_char_size(&self, face: u64, width26: i64, height26: i64) -> i32 {
        // SAFETY: the signature branch matches the qualified ABI.
        unsafe {
            if self.layout.long_bytes == 4 {
                let f: unsafe extern "C" fn(u64, i32, i32, u32, u32) -> i32 =
                    std::mem::transmute(self.ft_set_char_size);
                f(face, width26 as i32, height26 as i32, 72, 72)
            } else {
                let f: unsafe extern "C" fn(u64, i64, i64, u32, u32) -> i32 =
                    std::mem::transmute(self.ft_set_char_size);
                f(face, width26, height26, 72, 72)
            }
        }
    }

    /// # Safety
    ///
    /// `face` must be a live `FT_Face`.
    unsafe fn select_charmap(&self, face: u64, encoding: u32) -> i32 {
        let f: unsafe extern "C" fn(u64, u32) -> i32 =
            // SAFETY: the signature matches FreeType on every qualified ABI.
            unsafe { std::mem::transmute(self.ft_select_charmap) };
        // SAFETY: caller guarantees a live face.
        unsafe { f(face, encoding) }
    }

    /// # Safety
    ///
    /// `face` must be a live `FT_Face`.
    unsafe fn get_char_index(&self, face: u64, code: u32) -> u32 {
        // SAFETY: the signature branch matches the qualified ABI.
        unsafe {
            if self.layout.long_bytes == 4 {
                let f: unsafe extern "C" fn(u64, u32) -> u32 = std::mem::transmute(self.ft_get_char_index);
                f(face, code)
            } else {
                let f: unsafe extern "C" fn(u64, u64) -> u32 = std::mem::transmute(self.ft_get_char_index);
                f(face, u64::from(code))
            }
        }
    }

    /// # Safety
    ///
    /// `face` must be a live `FT_Face`.
    unsafe fn load_glyph(&self, face: u64, index: u32) -> i32 {
        let f: unsafe extern "C" fn(u64, u32, i32) -> i32 =
            // SAFETY: the signature matches FreeType on every qualified ABI.
            unsafe { std::mem::transmute(self.ft_load_glyph) };
        // SAFETY: caller guarantees a live face.
        unsafe { f(face, index, 0) }
    }

    /// # Safety
    ///
    /// `slot` must be a live `FT_GlyphSlot`.
    unsafe fn render_glyph(&self, slot: u64) -> i32 {
        let f: unsafe extern "C" fn(u64, i32) -> i32 =
            // SAFETY: the signature matches FreeType on every qualified ABI.
            unsafe { std::mem::transmute(self.ft_render_glyph) };
        // SAFETY: caller guarantees a live slot.
        unsafe { f(slot, 0) }
    }

    /// # Safety
    ///
    /// `outline` must be a live `FT_Outline*`.
    unsafe fn outline_translate(&self, outline: u64, dx: i64, dy: i64) {
        // SAFETY: the signature branch matches the qualified ABI.
        unsafe {
            if self.layout.long_bytes == 4 {
                let f: unsafe extern "C" fn(u64, i32, i32) = std::mem::transmute(self.ft_outline_translate);
                f(outline, dx as i32, dy as i32);
            } else {
                let f: unsafe extern "C" fn(u64, i64, i64) = std::mem::transmute(self.ft_outline_translate);
                f(outline, dx, dy);
            }
        }
    }

    /// # Safety
    ///
    /// `library`, `outline`, and `bitmap` must be live.
    unsafe fn outline_get_bitmap(&self, library: u64, outline: u64, bitmap: *mut u8) -> i32 {
        let f: unsafe extern "C" fn(u64, u64, *mut u8) -> i32 =
            // SAFETY: the signature matches FreeType on every qualified ABI.
            unsafe { std::mem::transmute(self.ft_outline_get_bitmap) };
        // SAFETY: caller guarantees live pointers.
        unsafe { f(library, outline, bitmap) }
    }
}

/// Library open outcome.
pub enum FreeTypeInitialization {
    /// No qualified ABI or library on this host.
    Unavailable {
        /// Cause.
        cause: Error,
    },
    /// The library loaded but initialization failed.
    Failed {
        /// Typed failure.
        error: FreeTypeError,
    },
    /// Ready for face creation.
    Ready {
        /// Open library.
        library: FreeTypeFontLibrary,
    },
}

/// Owns font bytes and native faces until release or library close.
pub struct FreeTypeFontLibrary {
    lib: Arc<FreeTypeLib>,
    layout: FreeTypeLayout,
    handle: u64,
    faces: HashMap<FreeTypeFace, FaceRecord>,
    next_face: u64,
}

impl FreeTypeFontLibrary {
    /// Open the library with live process discovery.
    pub fn open(print: impl Fn(&str)) -> Result<FreeTypeInitialization> {
        Self::open_with(print, &NativeLibraryOptions::default())
    }

    /// Open with explicit library discovery (tests inject overrides).
    pub fn open_with(print: impl Fn(&str), options: &NativeLibraryOptions) -> Result<FreeTypeInitialization> {
        let Some(layout) = host_free_type_layout() else {
            return Ok(FreeTypeInitialization::Unavailable {
                cause: Error::Unsupported("unsupported FreeType ABI".to_string()),
            });
        };
        // SAFETY: loading maps the image without invoking its code.
        let lib = match unsafe { FreeTypeLib::load(layout, options) } {
            Ok(lib) => Arc::new(lib),
            Err(error) => return Ok(FreeTypeInitialization::Unavailable { cause: error }),
        };
        let mut handle = 0u64;
        // SAFETY: the out-pointer is live.
        let code = unsafe { lib.init(&mut handle) };
        if code != 0 {
            print("R_InitFreeType: Unable to initialize FreeType.\n");
            return Ok(FreeTypeInitialization::Failed {
                error: FreeTypeError::new("FT_Init_FreeType", code),
            });
        }
        if handle == 0 {
            return Err(Error::InvalidInput("FreeType initialized a null library".to_string()));
        }
        Ok(FreeTypeInitialization::Ready {
            library: Self {
                lib,
                layout,
                handle,
                faces: HashMap::new(),
                next_face: 1,
            },
        })
    }

    /// Open face count.
    #[must_use]
    pub fn face_count(&self) -> usize {
        self.faces.len()
    }

    /// Whether the library is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.handle == 0
    }

    fn require_open(&self) -> Result<()> {
        if self.handle == 0 {
            return Err(Error::Closed("FreeType library".to_string()));
        }
        Ok(())
    }

    fn require_face(&self, face: FreeTypeFace) -> Result<&FaceRecord> {
        self.require_open()?;
        self.faces.get(&face).ok_or_else(|| {
            Error::InvalidInput("FreeType face is not owned by this library or was released".to_string())
        })
    }

    /// Create a face at `size` pixels, or `None` when the font is unusable.
    pub fn create_face(&mut self, bytes: &[u8], size: i32, print: impl Fn(&str)) -> Result<Option<FreeTypeFace>> {
        self.require_open()?;
        validate_glyph_size(size)?;
        if bytes.len() > i32::MAX as usize {
            return Err(Error::OutOfRange(
                "FreeType font exceeds signed long length".to_string(),
            ));
        }
        if bytes.is_empty() {
            print("RE_RegisterFont: FreeType2, unable to allocate new face.\n");
            return Ok(None);
        }
        let owned = bytes.to_vec();
        let mut address = 0u64;
        // SAFETY: the library is live; `owned` outlives the face via the record.
        let code = unsafe {
            self.lib
                .new_memory_face(self.handle, owned.as_ptr(), owned.len(), 0, &mut address)
        };
        if code != 0 {
            print("RE_RegisterFont: FreeType2, unable to allocate new face.\n");
            return Ok(None);
        }
        if address == 0 {
            return Err(Error::InvalidInput("FreeType created a null face".to_string()));
        }
        let face = FreeTypeFace { id: self.next_face };
        self.next_face += 1;
        self.faces.insert(face, FaceRecord { address, bytes: owned });
        // Unicode is selected explicitly so code points never use a symbol charmap.
        // SAFETY: the face is live.
        let charmap_error = unsafe { self.lib.select_charmap(address, 0x756e_6963) };
        // SAFETY: the face is live.
        let size_error = if charmap_error == 0 {
            unsafe {
                self.lib
                    .set_char_size(address, i64::from(size) * 64, i64::from(size) * 64)
            }
        } else {
            0
        };
        if charmap_error != 0 || size_error != 0 {
            self.release_face_quiet(face);
            if charmap_error != 0 {
                print(&format!("FreeType Unicode charmap unavailable ({charmap_error}).\n"));
            } else {
                print("RE_RegisterFont: FreeType2, Unable to set face char size.\n");
            }
            return Ok(None);
        }
        Ok(Some(face))
    }

    /// Glyph index for a code point (0 when the face lacks the glyph).
    pub fn glyph_index(&self, face: FreeTypeFace, code: u32) -> Result<u32> {
        let record = self.require_face(face)?;
        validate_code_point(code)?;
        // SAFETY: the face is live.
        Ok(unsafe { self.lib.get_char_index(record.address, code) })
    }

    /// Q3 outline raster contract, including padded pitch and source top/xSkip.
    pub fn render_glyph(&self, face: FreeTypeFace, code: u32, print: impl Fn(&str)) -> Result<Option<FontGlyphBitmap>> {
        let Some(slot) = self.load_glyph(face, code, &print)? else {
            return Ok(None);
        };
        let glyph = self.read(slot, self.layout.slot_format + 4)?;
        if u32::from_le_bytes(
            glyph[self.layout.slot_format..self.layout.slot_format + 4]
                .try_into()
                .expect("format"),
        ) != 0x6f75_746c
        {
            print("Non-outline fonts are not supported\n");
            return Ok(None);
        }
        let metric = |index: usize| free_type_metric(&glyph, 48 + index * self.layout.long_bytes, self.layout);
        let width26 = metric(0)?;
        let height26 = metric(1)?;
        let bearing_x = metric(2)?;
        let bearing_y = metric(3)?;
        let outline = outline_box(width26, height26, bearing_x, bearing_y)?;
        let length = free_type_bitmap_length(
            i64::from(outline.width),
            i64::from(outline.height),
            i64::from(outline.pitch),
            2,
        )?;
        let mut pixels = vec![0u8; length.max(1)];
        let mut bitmap = [0u8; 40];
        bitmap[0..4].copy_from_slice(&(outline.height as u32).to_le_bytes());
        bitmap[4..8].copy_from_slice(&(outline.width as u32).to_le_bytes());
        bitmap[8..12].copy_from_slice(&outline.pitch.to_le_bytes());
        bitmap[16..24].copy_from_slice(&(pixels.as_mut_ptr() as u64).to_le_bytes());
        bitmap[24..26].copy_from_slice(&256u16.to_le_bytes());
        bitmap[26] = 2;
        // SAFETY: the outline pointer designates the live glyph outline.
        let error = unsafe {
            self.lib.outline_translate(
                slot + self.layout.slot_outline as u64,
                i64::from(-outline.left),
                i64::from(-outline.bottom),
            );
            if length == 0 {
                0
            } else {
                self.lib
                    .outline_get_bitmap(self.handle, slot + self.layout.slot_outline as u64, bitmap.as_mut_ptr())
            }
        };
        if error != 0 {
            print(&format!("FT_Outline_Get_Bitmap failed ({error}).\n"));
            return Ok(None);
        }
        pixels.truncate(length);
        Ok(Some(FontGlyphBitmap {
            width: outline.width,
            height: outline.height,
            pitch: outline.pitch,
            top: bearing_y.div_euclid(64) + 1,
            bottom: outline.bottom,
            x_skip: metric(4)?.div_euclid(64) + 1,
            pixels,
        }))
    }

    /// FreeType's normal renderer; rows are copied before the reusable slot changes.
    pub fn render_bitmap(
        &self,
        face: FreeTypeFace,
        code: u32,
        print: impl Fn(&str),
    ) -> Result<Option<NativeFontGlyphBitmap>> {
        let Some(slot) = self.load_glyph(face, code, &print)? else {
            return Ok(None);
        };
        // SAFETY: the slot is live.
        let error = unsafe { self.lib.render_glyph(slot) };
        if error != 0 {
            print(&format!("FT_Render_Glyph failed ({error}).\n"));
            return Ok(None);
        }
        let record = self.read(slot, self.layout.slot_outline)?;
        let offset = self.layout.slot_bitmap;
        let height = u32::from_le_bytes(record[offset..offset + 4].try_into().expect("bitmap"));
        let width = u32::from_le_bytes(record[offset + 4..offset + 8].try_into().expect("bitmap"));
        let native_pitch = i32::from_le_bytes(record[offset + 8..offset + 12].try_into().expect("bitmap"));
        let pixel_mode = record[offset + 26];
        let num_grays = u16::from_le_bytes(record[offset + 24..offset + 26].try_into().expect("bitmap"));
        let length = free_type_bitmap_length(i64::from(width), i64::from(height), i64::from(native_pitch), pixel_mode)?;
        let address = u64::from_le_bytes(record[offset + 16..offset + 24].try_into().expect("bitmap"));
        let mut pixels = vec![0u8; length];
        if length != 0 {
            let start = if native_pitch < 0 {
                let start = i128::from(address) + (i128::from(height) - 1) * i128::from(native_pitch);
                u64::try_from(start)
                    .map_err(|_| Error::OutOfRange("invalid FreeType native memory range".to_string()))?
            } else {
                address
            };
            self.copy(&mut pixels, start)?;
            normalize_free_type_bitmap_rows(&mut pixels, height as usize, i64::from(native_pitch))?;
        }
        let advance = self.layout.slot_format - 2 * self.layout.long_bytes;
        Ok(Some(NativeFontGlyphBitmap {
            width,
            height,
            pitch: native_pitch.abs(),
            native_pitch,
            pixel_mode,
            num_grays,
            left: i32::from_le_bytes(record[offset + 40..offset + 44].try_into().expect("bitmap")),
            top: i32::from_le_bytes(record[offset + 44..offset + 48].try_into().expect("bitmap")),
            advance_x26: free_type_metric(&record, advance, self.layout)?,
            advance_y26: free_type_metric(&record, advance + self.layout.long_bytes, self.layout)?,
            pixels,
        }))
    }

    /// Release one face.
    pub fn release_face(&mut self, face: FreeTypeFace) -> Result<()> {
        let record = self.require_face(face)?;
        let address = record.address;
        // SAFETY: the face is live.
        let error = unsafe { self.lib.done_face(address) };
        self.faces.remove(&face);
        if error != 0 {
            return Err(FreeTypeError::new("FT_Done_Face", error).into());
        }
        Ok(())
    }

    fn release_face_quiet(&mut self, face: FreeTypeFace) {
        if let Some(record) = self.faces.remove(&face) {
            // SAFETY: the face is live.
            unsafe {
                self.lib.done_face(record.address);
            }
        }
    }

    /// Close the library, releasing every face. Idempotent.
    pub fn close(&mut self) -> Result<()> {
        if self.handle == 0 {
            return Ok(());
        }
        let mut errors = Vec::new();
        for record in self.faces.values() {
            // SAFETY: every recorded face is live.
            let code = unsafe { self.lib.done_face(record.address) };
            if code != 0 {
                errors.push(Error::from(FreeTypeError::new("FT_Done_Face", code)));
            }
        }
        // SAFETY: the library is live.
        let code = unsafe { self.lib.done(self.handle) };
        if code != 0 {
            errors.push(Error::from(FreeTypeError::new("FT_Done_FreeType", code)));
        }
        self.handle = 0;
        self.faces.clear();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(Error::aggregate("FreeType cleanup failed", errors))
        }
    }

    fn load_glyph(&self, face: FreeTypeFace, code: u32, print: &impl Fn(&str)) -> Result<Option<u64>> {
        let index = self.glyph_index(face, code)?;
        let record = self.require_face(face)?;
        // SAFETY: the face is live.
        let error = unsafe { self.lib.load_glyph(record.address, index) };
        if error != 0 {
            print(&format!("FT_Load_Glyph failed ({error}).\n"));
            return Ok(None);
        }
        let view = self.read(record.address, self.layout.face_glyph + 8)?;
        let slot = u64::from_le_bytes(
            view[self.layout.face_glyph..self.layout.face_glyph + 8]
                .try_into()
                .expect("glyph slot"),
        );
        if slot == 0 {
            return Err(Error::InvalidInput("FreeType face has no glyph slot".to_string()));
        }
        Ok(Some(slot))
    }

    fn copy(&self, bytes: &mut [u8], address: u64) -> Result<()> {
        if address == 0 || address.checked_add(bytes.len() as u64).is_none() {
            return Err(Error::OutOfRange("invalid FreeType native memory range".to_string()));
        }
        if bytes.is_empty() {
            return Ok(());
        }
        // SAFETY: the range was validated; FreeType owns the source.
        unsafe {
            std::ptr::copy_nonoverlapping(address as *const u8, bytes.as_mut_ptr(), bytes.len());
        }
        Ok(())
    }

    fn read(&self, address: u64, length: usize) -> Result<Vec<u8>> {
        let mut bytes = vec![0u8; length];
        self.copy(&mut bytes, address)?;
        Ok(bytes)
    }
}

impl Drop for FreeTypeFontLibrary {
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
            "QUAKE_FREETYPE_LIBRARY".to_string(),
            "/nonexistent-qa-platform/libfreetype.so".to_string(),
        );
        NativeLibraryOptions {
            environment: Some(environment),
            ..NativeLibraryOptions::default()
        }
    }

    #[test]
    fn validation_helpers() {
        assert!(validate_glyph_size(16).is_ok());
        assert!(validate_glyph_size(0).is_err());
        assert!(validate_glyph_size(0x0200_0000).is_err());
        assert!(validate_code_point(0x41).is_ok());
        assert!(validate_code_point(0x10_ffff).is_ok());
        assert!(validate_code_point(0xd800).is_err());
        assert!(validate_code_point(0x110000).is_err());
    }

    #[test]
    fn outline_box_rounds_like_source() {
        // bearingX=10, width=100 -> left=0, right=128; bearingY=70, height=50
        // -> top=128, bottom=0; width=2px, height=2px, pitch=4.
        let outline = outline_box(100, 50, 10, 70).unwrap();
        assert_eq!(outline.left, 0);
        assert_eq!(outline.right, 128);
        assert_eq!(outline.top, 128);
        assert_eq!(outline.bottom, 0);
        assert_eq!((outline.width, outline.height, outline.pitch), (2, 2, 4));
        assert!(outline_box(-1, 10, 0, 64).is_err());
        let empty = outline_box(0, 0, 0, 0).unwrap();
        assert_eq!((empty.width, empty.height), (0, 0));
    }

    #[test]
    fn missing_library_reports_unavailable() {
        match FreeTypeFontLibrary::open_with(|_| {}, &missing_lib()).expect("open handles absence") {
            FreeTypeInitialization::Unavailable { cause } => {
                assert!(cause.is_unavailable(), "{cause}");
                assert!(cause.to_string().contains("freetype"), "{cause}");
            }
            FreeTypeInitialization::Failed { error } => panic!("unexpected failure: {error}"),
            FreeTypeInitialization::Ready { .. } => panic!("missing library opened"),
        }
    }

    #[test]
    fn error_displays_operation_and_code() {
        let error = FreeTypeError::new("FT_Load_Glyph", 81);
        assert_eq!(error.to_string(), "FT_Load_Glyph failed with FreeType error 81");
        let converted = Error::from(error);
        assert!(converted.to_string().contains("FT_Load_Glyph"));
    }

    #[test]
    fn live_library_reports_honestly() {
        let candidates = [
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        ];
        let font = candidates.iter().find_map(|path| std::fs::read(path).ok());
        match FreeTypeFontLibrary::open(|_| {}).expect("open handles absence") {
            FreeTypeInitialization::Ready { mut library } => {
                let Some(font) = font else {
                    library.close().ok();
                    return;
                };
                let face = library.create_face(&font, 16, |_| {}).unwrap().expect("face");
                assert_eq!(library.face_count(), 1);
                assert!(library.glyph_index(face, u32::from(b'A')).unwrap() != 0);
                let outline = library.render_glyph(face, u32::from(b'A'), |_| {}).unwrap();
                assert!(outline.is_some());
                let bitmap = library.render_bitmap(face, u32::from(b'A'), |_| {}).unwrap();
                assert!(bitmap.is_some());
                library.release_face(face).unwrap();
                assert_eq!(library.face_count(), 0);
                library.close().unwrap();
                assert!(library.is_closed());
            }
            FreeTypeInitialization::Unavailable { cause } => {
                assert!(!cause.to_string().is_empty(), "{cause}");
            }
            FreeTypeInitialization::Failed { error } => {
                assert!(!error.to_string().is_empty());
            }
        }
    }
}
