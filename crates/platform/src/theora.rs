//! Theora video decoding through libtheoradec.
//!
//! Port of donor `src/platform/theora.ts`: Xiph's public LP64 API with
//! compressed packet framing in this module. Only the qualified 64-bit LP64
//! ABI (Linux/macOS) is supported.

use std::ffi::c_void;
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::ffi_util::LoadedLibrary;
use crate::native_libraries::{NativeLibrary, NativeLibraryOptions};

/// True on the qualified 64-bit LP64 ABI.
#[must_use]
pub fn theora_abi_supported() -> bool {
    #[cfg(all(
        any(target_os = "linux", target_os = "macos"),
        any(target_arch = "x86_64", target_arch = "aarch64")
    ))]
    {
        true
    }
    #[cfg(not(all(
        any(target_os = "linux", target_os = "macos"),
        any(target_arch = "x86_64", target_arch = "aarch64")
    )))]
    {
        false
    }
}

/// An Ogg packet (framing mirror of the media layer's packet).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OggPacket {
    /// Packet payload.
    pub data: Vec<u8>,
    /// Beginning-of-stream packet.
    pub first: bool,
    /// End-of-stream packet.
    pub last: bool,
    /// Granule position.
    pub granule: i64,
    /// Packet index.
    pub index: u64,
}

/// Encode the 48-byte LP64 `ogg_packet`: pointer, three C longs, granulepos
/// and packetno, all eight-byte fields.
pub fn native_packet_bytes(packet: &OggPacket, data_ptr: u64) -> [u8; 48] {
    let mut storage = [0u8; 48];
    storage[0..8].copy_from_slice(&(if packet.data.is_empty() { 0 } else { data_ptr }).to_le_bytes());
    storage[8..16].copy_from_slice(&(packet.data.len() as i64).to_le_bytes());
    storage[16..24].copy_from_slice(&i64::from(packet.first).to_le_bytes());
    storage[24..32].copy_from_slice(&i64::from(packet.last).to_le_bytes());
    storage[32..40].copy_from_slice(&packet.granule.to_le_bytes());
    storage[40..48].copy_from_slice(&(packet.index as i64).to_le_bytes());
    storage
}

/// Decoded picture: owned RGBA pixels, row-major top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TheoraPicture {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// RGBA pixels.
    pub rgba: Vec<u8>,
}

/// Validated Theora stream parameters from `th_info`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TheoraStreamInfo {
    /// Picture width.
    pub width: u32,
    /// Picture height.
    pub height: u32,
    /// Horizontal crop offset.
    pub crop_x: u32,
    /// Vertical crop offset.
    pub crop_y: u32,
    /// Frame duration in milliseconds.
    pub frame_ms: f64,
    /// Pixel format (0, 2, or 3).
    pub pixel_format: i32,
}

/// Parse and validate the 64-byte `th_info` record.
pub fn theora_stream_info(info: &[u8; 64]) -> Result<TheoraStreamInfo> {
    let field = |offset: usize| u32::from_le_bytes(info[offset..offset + 4].try_into().expect("info"));
    let frame_width = field(4);
    let frame_height = field(8);
    let width = field(12);
    let height = field(16);
    let crop_x = field(20);
    let crop_y = field(24);
    let numerator = field(28);
    let denominator = field(32);
    let pixel_format = i32::from_le_bytes(info[48..52].try_into().expect("info"));
    if width < 1
        || height < 1
        || u64::from(frame_width) * u64::from(frame_height) > 0x100_0000
        || crop_x + width > frame_width
        || crop_y + height > frame_height
        || numerator == 0
        || denominator == 0
        || ![0, 2, 3].contains(&pixel_format)
    {
        return Err(Error::InvalidInput(
            "unsupported Theora dimensions, rate or pixel format".to_string(),
        ));
    }
    let frame_ms = f64::from(denominator) * 1000.0 / f64::from(numerator);
    if !(1.0..=10000.0).contains(&frame_ms) {
        return Err(Error::InvalidInput(
            "Theora frame rate outside supported limits".to_string(),
        ));
    }
    Ok(TheoraStreamInfo {
        width,
        height,
        crop_x,
        crop_y,
        frame_ms,
        pixel_format,
    })
}

/// One decoded YCbCr plane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TheoraPlane {
    /// Plane width.
    pub width: u32,
    /// Plane height.
    pub height: u32,
    /// Row stride.
    pub stride: u32,
    /// Plane bytes (`stride * height`).
    pub bytes: Vec<u8>,
}

/// Convert cropped YCbCr planes to RGBA with the donor's exact coefficients.
pub fn ycbcr_to_rgba(
    y_plane: &TheoraPlane,
    cb: &TheoraPlane,
    cr: &TheoraPlane,
    info: &TheoraStreamInfo,
) -> Result<TheoraPicture> {
    let (width, height) = (info.width as usize, info.height as usize);
    let (crop_x, crop_y) = (info.crop_x as usize, info.crop_y as usize);
    let shift_x = usize::from(info.pixel_format != 3);
    let shift_y = usize::from(info.pixel_format == 0);
    if crop_x + width > y_plane.width as usize
        || crop_y + height > y_plane.height as usize
        || ((crop_x + width - 1) >> shift_x) >= cb.width as usize
        || ((crop_y + height - 1) >> shift_y) >= cb.height as usize
        || cb.width != cr.width
        || cb.height != cr.height
    {
        return Err(Error::InvalidInput("Theora crop exceeds decoded planes".to_string()));
    }
    let sample = |plane: &TheoraPlane, x: usize, y: usize| -> Result<u8> {
        plane
            .bytes
            .get(y * plane.stride as usize + x)
            .copied()
            .ok_or_else(|| Error::InvalidInput("missing Theora color sample".to_string()))
    };
    let byte = |value: f64| value.round().clamp(0.0, 255.0) as u8;
    let mut rgba = vec![0u8; width * height * 4];
    for y in 0..height {
        for x in 0..width {
            let source_x = crop_x + x;
            let source_y = crop_y + y;
            let luma = sample(y_plane, source_x, source_y)?;
            let blue = sample(cb, source_x >> shift_x, source_y >> shift_y)?;
            let red = sample(cr, source_x >> shift_x, source_y >> shift_y)?;
            let yy = 1.1643835616438356 * (f64::from(luma) - 16.0);
            let u = f64::from(blue) - 128.0;
            let v = f64::from(red) - 128.0;
            let offset = (y * width + x) * 4;
            rgba[offset] = byte(yy + 1.5960267857142858 * v);
            rgba[offset + 1] = byte(yy - 0.39176229009491365 * u - 0.8129676472377708 * v);
            rgba[offset + 2] = byte(yy + 2.017232142857143 * u);
            rgba[offset + 3] = 255;
        }
    }
    Ok(TheoraPicture {
        width: info.width,
        height: info.height,
        rgba,
    })
}

struct TheoraLib {
    _lib: LoadedLibrary,
    th_info_init: unsafe extern "C" fn(*mut u8),
    th_info_clear: unsafe extern "C" fn(*mut u8),
    th_comment_init: unsafe extern "C" fn(*mut u8),
    th_comment_clear: unsafe extern "C" fn(*mut u8),
    th_decode_headerin: unsafe extern "C" fn(*mut u8, *mut u8, *mut u64, *const u8) -> i32,
    th_decode_alloc: unsafe extern "C" fn(*const u8, u64) -> *mut c_void,
    th_setup_free: unsafe extern "C" fn(u64),
    th_decode_free: unsafe extern "C" fn(*mut c_void),
    th_decode_packetin: unsafe extern "C" fn(*mut c_void, *const u8, *mut i64) -> i32,
    th_decode_ycbcr_out: unsafe extern "C" fn(*mut c_void, *mut u8) -> i32,
}

impl TheoraLib {
    /// # Safety
    ///
    /// Resolved symbols are only invoked with the libtheoradec ABI below.
    unsafe fn load(options: &NativeLibraryOptions) -> Result<Self> {
        if !theora_abi_supported() {
            return Err(Error::Unsupported(
                "Theora decoding requires the qualified 64-bit LP64 ABI".to_string(),
            ));
        }
        // SAFETY: loading maps the image without invoking its code.
        let lib = unsafe { LoadedLibrary::open(NativeLibrary::TheoraDec, options)? };
        macro_rules! sym {
            ($name:literal, $sig:ty) => {
                // SAFETY: the address is only read here.
                unsafe { lib.symbol::<$sig>(concat!($name, "\0").as_bytes())? }
            };
        }
        Ok(Self {
            th_info_init: sym!("th_info_init", unsafe extern "C" fn(*mut u8)),
            th_info_clear: sym!("th_info_clear", unsafe extern "C" fn(*mut u8)),
            th_comment_init: sym!("th_comment_init", unsafe extern "C" fn(*mut u8)),
            th_comment_clear: sym!("th_comment_clear", unsafe extern "C" fn(*mut u8)),
            th_decode_headerin: sym!(
                "th_decode_headerin",
                unsafe extern "C" fn(*mut u8, *mut u8, *mut u64, *const u8) -> i32
            ),
            th_decode_alloc: sym!("th_decode_alloc", unsafe extern "C" fn(*const u8, u64) -> *mut c_void),
            th_setup_free: sym!("th_setup_free", unsafe extern "C" fn(u64)),
            th_decode_free: sym!("th_decode_free", unsafe extern "C" fn(*mut c_void)),
            th_decode_packetin: sym!(
                "th_decode_packetin",
                unsafe extern "C" fn(*mut c_void, *const u8, *mut i64) -> i32
            ),
            th_decode_ycbcr_out: sym!("th_decode_ycbcr_out", unsafe extern "C" fn(*mut c_void, *mut u8) -> i32),
            _lib: lib,
        })
    }
}

// SAFETY: symbols are only called on the owning thread.
unsafe impl Send for TheoraLib {}
unsafe impl Sync for TheoraLib {}

/// An open Theora decoder.
pub struct TheoraDecoder {
    lib: Arc<TheoraLib>,
    context: *mut c_void,
    info: TheoraStreamInfo,
}

// SAFETY: the decoder is only used on the owning thread.
unsafe impl Send for TheoraDecoder {}

impl TheoraDecoder {
    /// Open a decoder over three header packets with live process discovery.
    pub fn new(headers: &[OggPacket]) -> Result<Self> {
        Self::new_with(headers, &NativeLibraryOptions::default())
    }

    /// Open with explicit library discovery (tests inject overrides).
    pub fn new_with(headers: &[OggPacket], options: &NativeLibraryOptions) -> Result<Self> {
        // SAFETY: loading maps the image; calls below use validated arguments.
        let lib = Arc::new(unsafe { TheoraLib::load(options)? });
        // SAFETY: info/comment/setup buffers are live for the header calls.
        unsafe {
            let mut info = [0u8; 64];
            let mut comments = [0u8; 32];
            let mut setup = 0u64;
            (lib.th_info_init)(info.as_mut_ptr());
            (lib.th_comment_init)(comments.as_mut_ptr());
            let outcome = (|| {
                if headers.len() != 3 {
                    return Err(Error::InvalidInput("Theora requires three header packets".to_string()));
                }
                for packet in headers {
                    let data_ptr = packet.data.as_ptr() as u64;
                    let storage = native_packet_bytes(packet, data_ptr);
                    let result = (lib.th_decode_headerin)(
                        info.as_mut_ptr(),
                        comments.as_mut_ptr(),
                        &mut setup,
                        storage.as_ptr(),
                    );
                    if result <= 0 {
                        return Err(Error::InvalidInput(format!("invalid Theora header: {result}")));
                    }
                }
                let parsed = theora_stream_info(&info)?;
                if setup == 0 {
                    return Err(Error::InvalidInput("Theora setup allocation failed".to_string()));
                }
                let context = (lib.th_decode_alloc)(info.as_ptr(), setup);
                setup = 0;
                if context.is_null() {
                    return Err(Error::InvalidInput("Theora decoder allocation failed".to_string()));
                }
                Ok((parsed, context))
            })();
            if setup != 0 {
                (lib.th_setup_free)(setup);
            }
            (lib.th_comment_clear)(comments.as_mut_ptr());
            (lib.th_info_clear)(info.as_mut_ptr());
            let (parsed, context) = outcome?;
            Ok(Self {
                lib,
                context,
                info: parsed,
            })
        }
    }

    /// Picture width.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.info.width
    }

    /// Picture height.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.info.height
    }

    /// Frame duration in milliseconds.
    #[must_use]
    pub fn frame_ms(&self) -> f64 {
        self.info.frame_ms
    }

    /// Whether the decoder is closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.context.is_null()
    }

    /// Decode one video packet into an owned RGBA picture.
    pub fn decode(&mut self, packet: &OggPacket) -> Result<TheoraPicture> {
        if self.context.is_null() {
            return Err(Error::Closed("Theora decoder".to_string()));
        }
        // SAFETY: the context is live; packet and plane buffers are live per call.
        unsafe {
            let data_ptr = packet.data.as_ptr() as u64;
            let storage = native_packet_bytes(packet, data_ptr);
            let mut granule = 0i64;
            let mut planes = [0u8; 72];
            let result = (self.lib.th_decode_packetin)(self.context, storage.as_ptr(), &mut granule);
            if result < 0 {
                return Err(Error::InvalidInput(format!("invalid Theora frame: {result}")));
            }
            if (self.lib.th_decode_ycbcr_out)(self.context, planes.as_mut_ptr()) != 0 {
                return Err(Error::InvalidInput("Theora frame planes unavailable".to_string()));
            }
            let y_plane = Self::plane(&planes, 0)?;
            let cb = Self::plane(&planes, 1)?;
            let cr = Self::plane(&planes, 2)?;
            ycbcr_to_rgba(&y_plane, &cb, &cr, &self.info)
        }
    }

    /// # Safety
    ///
    /// `planes` must be the 72-byte output of `th_decode_ycbcr_out`.
    unsafe fn plane(planes: &[u8; 72], index: usize) -> Result<TheoraPlane> {
        let offset = index * 24;
        let width = i32::from_le_bytes(planes[offset..offset + 4].try_into().expect("plane"));
        let height = i32::from_le_bytes(planes[offset + 4..offset + 8].try_into().expect("plane"));
        let stride = i32::from_le_bytes(planes[offset + 8..offset + 12].try_into().expect("plane"));
        let pointer = u64::from_le_bytes(planes[offset + 16..offset + 24].try_into().expect("plane"));
        if pointer == 0
            || width < 1
            || height < 1
            || stride < width
            || i64::from(stride) * i64::from(height) > 128 * 1024 * 1024
        {
            return Err(Error::InvalidInput("invalid Theora output plane".to_string()));
        }
        let mut bytes = vec![0u8; stride as usize * height as usize];
        // SAFETY: the plane range was validated; libtheoradec owns the source.
        unsafe {
            std::ptr::copy_nonoverlapping(pointer as *const u8, bytes.as_mut_ptr(), bytes.len());
        }
        Ok(TheoraPlane {
            width: width as u32,
            height: height as u32,
            stride: stride as u32,
            bytes,
        })
    }

    /// Close the decoder. Idempotent.
    pub fn close(&mut self) {
        if self.context.is_null() {
            return;
        }
        // SAFETY: the context is live until this call.
        unsafe {
            (self.lib.th_decode_free)(self.context);
        }
        self.context = std::ptr::null_mut();
    }
}

impl Drop for TheoraDecoder {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn missing_lib() -> NativeLibraryOptions {
        let mut environment = std::collections::HashMap::new();
        environment.insert(
            "QUAKE_THEORA_LIBRARY".to_string(),
            "/nonexistent-qa-platform/libtheoradec.so".to_string(),
        );
        NativeLibraryOptions {
            environment: Some(environment),
            ..NativeLibraryOptions::default()
        }
    }

    fn header_packets() -> Vec<OggPacket> {
        (0..3)
            .map(|index| OggPacket {
                data: vec![1, 2, 3],
                first: index == 0,
                last: false,
                granule: 0,
                index,
            })
            .collect()
    }

    #[test]
    fn packet_layout_matches_lp64() {
        let packet = OggPacket {
            data: vec![9; 5],
            first: true,
            last: false,
            granule: -7,
            index: 3,
        };
        let bytes = native_packet_bytes(&packet, 0x1234_5678);
        assert_eq!(u64::from_le_bytes(bytes[0..8].try_into().unwrap()), 0x1234_5678);
        assert_eq!(i64::from_le_bytes(bytes[8..16].try_into().unwrap()), 5);
        assert_eq!(i64::from_le_bytes(bytes[16..24].try_into().unwrap()), 1);
        assert_eq!(i64::from_le_bytes(bytes[24..32].try_into().unwrap()), 0);
        assert_eq!(i64::from_le_bytes(bytes[32..40].try_into().unwrap()), -7);
        assert_eq!(i64::from_le_bytes(bytes[40..48].try_into().unwrap()), 3);
        let empty = OggPacket {
            data: Vec::new(),
            first: false,
            last: true,
            granule: 0,
            index: 0,
        };
        assert_eq!(
            u64::from_le_bytes(native_packet_bytes(&empty, 0x99)[0..8].try_into().unwrap()),
            0
        );
    }

    #[test]
    fn stream_info_validates() {
        let mut info = [0u8; 64];
        info[4..8].copy_from_slice(&320u32.to_le_bytes());
        info[8..12].copy_from_slice(&240u32.to_le_bytes());
        info[12..16].copy_from_slice(&320u32.to_le_bytes());
        info[16..20].copy_from_slice(&240u32.to_le_bytes());
        info[20..24].copy_from_slice(&0u32.to_le_bytes());
        info[24..28].copy_from_slice(&0u32.to_le_bytes());
        info[28..32].copy_from_slice(&30u32.to_le_bytes());
        info[32..36].copy_from_slice(&1u32.to_le_bytes());
        info[48..52].copy_from_slice(&0i32.to_le_bytes());
        let parsed = theora_stream_info(&info).unwrap();
        assert_eq!((parsed.width, parsed.height), (320, 240));
        assert!((parsed.frame_ms - 1000.0 / 30.0).abs() < 1e-9);
        info[48..52].copy_from_slice(&1i32.to_le_bytes());
        assert!(theora_stream_info(&info).is_err());
        info[48..52].copy_from_slice(&0i32.to_le_bytes());
        info[28..32].copy_from_slice(&0u32.to_le_bytes());
        assert!(theora_stream_info(&info).is_err());
        info[28..32].copy_from_slice(&30u32.to_le_bytes());
        info[12..16].copy_from_slice(&9999u32.to_le_bytes());
        assert!(theora_stream_info(&info).is_err());
        assert!(theora_abi_supported());
    }

    #[test]
    fn ycbcr_converts_gray_and_validates_crop() {
        // 4:2:0 gray: luma 235 (white), chroma 128 (neutral), 4x2 picture.
        let info = TheoraStreamInfo {
            width: 4,
            height: 2,
            crop_x: 0,
            crop_y: 0,
            frame_ms: 33.0,
            pixel_format: 0,
        };
        let y_plane = TheoraPlane {
            width: 4,
            height: 2,
            stride: 4,
            bytes: vec![235u8; 8],
        };
        let chroma = TheoraPlane {
            width: 2,
            height: 1,
            stride: 2,
            bytes: vec![128u8; 2],
        };
        let picture = ycbcr_to_rgba(&y_plane, &chroma, &chroma, &info).unwrap();
        assert_eq!((picture.width, picture.height), (4, 2));
        assert_eq!(picture.rgba.len(), 32);
        assert_eq!(&picture.rgba[0..4], &[255, 255, 255, 255]);
        // Black: luma 16 maps to 0.
        let black_y = TheoraPlane {
            bytes: vec![16u8; 8],
            ..y_plane.clone()
        };
        let black = ycbcr_to_rgba(&black_y, &chroma, &chroma, &info).unwrap();
        assert_eq!(&black.rgba[0..4], &[0, 0, 0, 255]);
        // Crop outside the planes fails.
        let bad_crop = TheoraStreamInfo { crop_x: 3, ..info };
        assert!(ycbcr_to_rgba(&y_plane, &chroma, &chroma, &bad_crop).is_err());
    }

    #[test]
    fn missing_library_names_theoradec() {
        let Err(error) = TheoraDecoder::new_with(&header_packets(), &missing_lib()) else {
            panic!("expected failure")
        };
        assert!(error.to_string().contains("theora"), "{error}");
    }

    #[test]
    fn live_headers_report_honestly() {
        // Synthetic headers are invalid for a real decoder (header error) or
        // unusable without the library (absence signal); either is honest.
        match TheoraDecoder::new(&header_packets()) {
            Ok(_) => panic!("synthetic headers decoded"),
            Err(error) => assert!(!error.to_string().is_empty(), "{error}"),
        }
    }
}
