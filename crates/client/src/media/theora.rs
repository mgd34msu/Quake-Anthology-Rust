//! Theora video decode over `libtheoradec`.
//!
//! Donor provenance: `src/platform/theora.ts` (Xiph `libtheoradec`
//! public LP64 API; compressed packet framing stays in
//! [`crate::media::containers`]).
//!
//! Only the qualified 64-bit LP64 ABI is supported: `ogg_packet` is
//! six eight-byte fields, and `th_info`/`th_comment` field offsets
//! match the donor's `DataView` reads.

use std::ffi::c_void;
use std::os::raw::{c_int, c_long};

use super::containers::OggPacket;
use crate::ClientError;

/// `ogg_packet` on LP64 (48 bytes).
#[repr(C)]
struct OggPacketFfi {
    packet: *const u8,
    bytes: c_long,
    b_o_s: c_long,
    e_o_s: c_long,
    granulepos: i64,
    packetno: i64,
}

/// `th_img_plane` on LP64 (24 bytes).
#[repr(C)]
struct ThImgPlane {
    width: c_int,
    height: c_int,
    stride: c_int,
    data: *mut u8,
}

#[link(name = "theoradec")]
extern "C" {
    fn th_info_init(info: *mut u8);
    fn th_info_clear(info: *mut u8);
    fn th_comment_init(comments: *mut u8);
    fn th_comment_clear(comments: *mut u8);
    fn th_decode_headerin(
        info: *mut u8,
        comments: *mut u8,
        setup: *mut *mut c_void,
        packet: *const OggPacketFfi,
    ) -> c_int;
    fn th_decode_alloc(info: *const u8, setup: *const c_void) -> *mut c_void;
    fn th_setup_free(setup: *mut c_void);
    fn th_decode_free(ctx: *mut c_void);
    fn th_decode_packetin(ctx: *mut c_void, packet: *const OggPacketFfi, granule: *mut i64) -> c_int;
    fn th_decode_ycbcr_out(ctx: *mut c_void, planes: *mut ThImgPlane) -> c_int;
}

/// A decoded Theora picture (`TheoraPicture`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TheoraPicture {
    /// Width.
    pub width: usize,
    /// Height.
    pub height: usize,
    /// RGBA pixels.
    pub rgba: Vec<u8>,
}

/// One decoded output plane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TheoraPlaneBytes {
    /// Plane bytes (`stride * height`).
    pub bytes: Vec<u8>,
    /// Width.
    pub width: usize,
    /// Height.
    pub height: usize,
    /// Stride.
    pub stride: usize,
}

fn native_packet(packet: &OggPacket) -> OggPacketFfi {
    OggPacketFfi {
        packet: if packet.data.is_empty() {
            std::ptr::null()
        } else {
            packet.data.as_ptr()
        },
        bytes: packet.data.len() as c_long,
        b_o_s: c_long::from(u8::from(packet.first)),
        e_o_s: c_long::from(u8::from(packet.last)),
        granulepos: packet.granule,
        packetno: packet.index as c_long,
    }
}

fn info_u32(info: &[u8; 64], offset: usize) -> u32 {
    u32::from_le_bytes([info[offset], info[offset + 1], info[offset + 2], info[offset + 3]])
}

fn info_i32(info: &[u8; 64], offset: usize) -> i32 {
    i32::from_le_bytes([info[offset], info[offset + 1], info[offset + 2], info[offset + 3]])
}

fn check_plane(width: c_int, height: c_int, stride: c_int) -> Result<(usize, usize, usize), ClientError> {
    if width < 1 || height < 1 || stride < width {
        return Err(ClientError::BadMedia("Invalid Theora output plane".to_string()));
    }
    if i64::from(stride) * i64::from(height) > 128 * 1024 * 1024 {
        return Err(ClientError::BadMedia("Invalid Theora output plane".to_string()));
    }
    Ok((width as usize, height as usize, stride as usize))
}

/// Borrowed decoded planes (luma plus chroma).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TheoraPlanes<'a> {
    /// Luma.
    pub y: &'a TheoraPlaneBytes,
    /// Chroma blue.
    pub cb: &'a TheoraPlaneBytes,
    /// Chroma red.
    pub cr: &'a TheoraPlaneBytes,
}

/// Assemble an RGBA picture from decoded planes (crop checks and
/// Y'CbCr conversion, without native calls).
pub fn theora_picture_from_planes(
    planes: TheoraPlanes<'_>,
    crop_x: usize,
    crop_y: usize,
    width: usize,
    height: usize,
    pixel_format: i32,
) -> Result<TheoraPicture, ClientError> {
    let TheoraPlanes { y: y_plane, cb, cr } = planes;
    let shift_x = if pixel_format == 3 { 0 } else { 1 };
    let shift_y = if pixel_format == 0 { 1 } else { 0 };
    if crop_x + width > y_plane.width
        || crop_y + height > y_plane.height
        || ((crop_x + width - 1) >> shift_x) >= cb.width
        || ((crop_y + height - 1) >> shift_y) >= cb.height
        || cb.width != cr.width
        || cb.height != cr.height
    {
        return Err(ClientError::BadMedia("Theora crop exceeds decoded planes".to_string()));
    }
    let byte = |value: f64| value.round().clamp(0.0, 255.0) as u8;
    let mut rgba = vec![0u8; width * height * 4];
    for y in 0..height {
        for x in 0..width {
            let source_x = crop_x + x;
            let source_y = crop_y + y;
            let luma = y_plane.bytes.get(source_y * y_plane.stride + source_x).copied();
            let blue = cb
                .bytes
                .get((source_y >> shift_y) * cb.stride + (source_x >> shift_x))
                .copied();
            let red = cr
                .bytes
                .get((source_y >> shift_y) * cr.stride + (source_x >> shift_x))
                .copied();
            let (Some(luma), Some(blue), Some(red)) = (luma, blue, red) else {
                return Err(ClientError::BadMedia("Missing Theora color sample".to_string()));
            };
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
    Ok(TheoraPicture { width, height, rgba })
}

/// A Theora decoder (`TheoraDecoder`).
pub struct TheoraDecoder {
    ctx: *mut c_void,
    width: usize,
    height: usize,
    frame_ms: f64,
    crop_x: usize,
    crop_y: usize,
    pixel_format: i32,
}

impl Drop for TheoraDecoder {
    fn drop(&mut self) {
        self.close();
    }
}

impl TheoraDecoder {
    /// Open a decoder over three header packets.
    pub fn new(headers: &[OggPacket]) -> Result<Self, ClientError> {
        let mut info = [0u8; 64];
        let mut comments = [0u8; 32];
        let mut setup: *mut c_void = std::ptr::null_mut();
        // SAFETY: the buffers match `th_info`/`th_comment` sizes and
        // stay alive across the synchronous calls.
        unsafe {
            th_info_init(info.as_mut_ptr());
            th_comment_init(comments.as_mut_ptr());
        }
        let result = Self::init_headers(headers, &mut info, &mut comments, &mut setup);
        // SAFETY: same buffers; freeing a null setup is skipped.
        unsafe {
            if !setup.is_null() {
                th_setup_free(setup);
            }
            th_comment_clear(comments.as_mut_ptr());
            th_info_clear(info.as_mut_ptr());
        }
        result
    }

    fn init_headers(
        headers: &[OggPacket],
        info: &mut [u8; 64],
        comments: &mut [u8; 32],
        setup: &mut *mut c_void,
    ) -> Result<Self, ClientError> {
        if headers.len() != 3 {
            return Err(ClientError::BadMedia(
                "Theora requires three header packets".to_string(),
            ));
        }
        for packet in headers {
            let native = native_packet(packet);
            // SAFETY: the packet borrows live movie bytes across the
            // synchronous call.
            let result = unsafe { th_decode_headerin(info.as_mut_ptr(), comments.as_mut_ptr(), setup, &native) };
            if result <= 0 {
                return Err(ClientError::BadMedia(format!("Invalid Theora header: {result}")));
            }
        }
        let frame_width = info_u32(info, 4);
        let frame_height = info_u32(info, 8);
        let width = info_u32(info, 12);
        let height = info_u32(info, 16);
        let crop_x = info_u32(info, 20);
        let crop_y = info_u32(info, 24);
        let numerator = info_u32(info, 28);
        let denominator = info_u32(info, 32);
        let pixel_format = info_i32(info, 48);
        if width < 1
            || height < 1
            || frame_width as u64 * frame_height as u64 > 0x1000000
            || crop_x + width > frame_width
            || crop_y + height > frame_height
            || numerator == 0
            || denominator == 0
            || ![0, 2, 3].contains(&pixel_format)
        {
            return Err(ClientError::BadMedia(
                "Unsupported Theora dimensions, rate or pixel format".to_string(),
            ));
        }
        let frame_ms = f64::from(denominator) * 1000.0 / f64::from(numerator);
        if frame_ms < 1.0 || frame_ms > 10000.0 {
            return Err(ClientError::BadMedia(
                "Theora frame rate outside supported limits".to_string(),
            ));
        }
        if setup.is_null() {
            return Err(ClientError::BadMedia("Theora setup allocation failed".to_string()));
        }
        // SAFETY: setup came from successful header decode.
        let ctx = unsafe { th_decode_alloc(info.as_ptr(), *setup) };
        if ctx.is_null() {
            return Err(ClientError::BadMedia("Theora decoder allocation failed".to_string()));
        }
        Ok(Self {
            ctx,
            width: width as usize,
            height: height as usize,
            frame_ms,
            crop_x: crop_x as usize,
            crop_y: crop_y as usize,
            pixel_format,
        })
    }

    /// Width.
    #[must_use]
    pub const fn width(&self) -> usize {
        self.width
    }

    /// Height.
    #[must_use]
    pub const fn height(&self) -> usize {
        self.height
    }

    /// Frame interval in milliseconds.
    #[must_use]
    pub const fn frame_ms(&self) -> f64 {
        self.frame_ms
    }

    /// Decode a frame packet (`decode`).
    pub fn decode(&mut self, packet: &OggPacket) -> Result<TheoraPicture, ClientError> {
        if self.ctx.is_null() {
            return Err(ClientError::BadMedia("Theora decoder is closed".to_string()));
        }
        let native = native_packet(packet);
        let mut granule = 0i64;
        // SAFETY: the context is live and the packet borrows live
        // movie bytes across the synchronous call.
        let result = unsafe { th_decode_packetin(self.ctx, &native, &mut granule) };
        if result < 0 {
            return Err(ClientError::BadMedia(format!("Invalid Theora frame: {result}")));
        }
        let mut planes = [
            ThImgPlane {
                width: 0,
                height: 0,
                stride: 0,
                data: std::ptr::null_mut(),
            },
            ThImgPlane {
                width: 0,
                height: 0,
                stride: 0,
                data: std::ptr::null_mut(),
            },
            ThImgPlane {
                width: 0,
                height: 0,
                stride: 0,
                data: std::ptr::null_mut(),
            },
        ];
        // SAFETY: the context is live and the plane array is a valid
        // `th_ycbcr_buffer` out-parameter.
        if unsafe { th_decode_ycbcr_out(self.ctx, planes.as_mut_ptr()) } != 0 {
            return Err(ClientError::BadMedia("Theora frame planes unavailable".to_string()));
        }
        let mut decoded = Vec::with_capacity(3);
        for plane in &planes {
            if plane.data.is_null() {
                return Err(ClientError::BadMedia("Invalid Theora output plane".to_string()));
            }
            let (width, height, stride) = check_plane(plane.width, plane.height, plane.stride)?;
            let mut bytes = vec![0u8; stride * height];
            // SAFETY: the decoder wrote `stride * height` readable
            // bytes at `data`, which stays alive across this copy.
            unsafe {
                std::ptr::copy_nonoverlapping(plane.data, bytes.as_mut_ptr(), bytes.len());
            }
            decoded.push(TheoraPlaneBytes {
                bytes,
                width,
                height,
                stride,
            });
        }
        theora_picture_from_planes(
            TheoraPlanes {
                y: &decoded[0],
                cb: &decoded[1],
                cr: &decoded[2],
            },
            self.crop_x,
            self.crop_y,
            self.width,
            self.height,
            self.pixel_format,
        )
    }

    /// Close the decoder.
    pub fn close(&mut self) {
        if !self.ctx.is_null() {
            // SAFETY: the context is live and used once.
            unsafe {
                th_decode_free(self.ctx);
            }
            self.ctx = std::ptr::null_mut();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, offset_of, size_of};

    #[test]
    fn ffi_layouts_match_lp64() {
        assert_eq!(size_of::<OggPacketFfi>(), 48);
        assert_eq!(align_of::<OggPacketFfi>(), 8);
        assert_eq!(offset_of!(OggPacketFfi, packet), 0);
        assert_eq!(offset_of!(OggPacketFfi, bytes), 8);
        assert_eq!(offset_of!(OggPacketFfi, b_o_s), 16);
        assert_eq!(offset_of!(OggPacketFfi, e_o_s), 24);
        assert_eq!(offset_of!(OggPacketFfi, granulepos), 32);
        assert_eq!(offset_of!(OggPacketFfi, packetno), 40);
        assert_eq!(size_of::<ThImgPlane>(), 24);
        assert_eq!(offset_of!(ThImgPlane, width), 0);
        assert_eq!(offset_of!(ThImgPlane, height), 4);
        assert_eq!(offset_of!(ThImgPlane, stride), 8);
        assert_eq!(offset_of!(ThImgPlane, data), 16);
    }

    fn planes(luma: u8, blue: u8, red: u8) -> (TheoraPlaneBytes, TheoraPlaneBytes, TheoraPlaneBytes) {
        (
            TheoraPlaneBytes {
                bytes: vec![luma; 16],
                width: 4,
                height: 4,
                stride: 4,
            },
            TheoraPlaneBytes {
                bytes: vec![blue; 4],
                width: 2,
                height: 2,
                stride: 2,
            },
            TheoraPlaneBytes {
                bytes: vec![red; 4],
                width: 2,
                height: 2,
                stride: 2,
            },
        )
    }

    #[test]
    fn ycbcr_conversion_matches_donor() {
        // Black and white anchors plus a hand-computed color.
        let (y, cb, cr) = planes(16, 128, 128);
        let picture = theora_picture_from_planes(
            TheoraPlanes {
                y: &y,
                cb: &cb,
                cr: &cr,
            },
            0,
            0,
            4,
            4,
            0,
        )
        .unwrap();
        assert!(picture
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| *pixel == [0, 0, 0, 255]));
        let (y, cb, cr) = planes(235, 128, 128);
        let picture = theora_picture_from_planes(
            TheoraPlanes {
                y: &y,
                cb: &cb,
                cr: &cr,
            },
            0,
            0,
            4,
            4,
            0,
        )
        .unwrap();
        assert!(picture
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| *pixel == [255, 255, 255, 255]));
        let (y, cb, cr) = planes(81, 90, 240);
        let picture = theora_picture_from_planes(
            TheoraPlanes {
                y: &y,
                cb: &cb,
                cr: &cr,
            },
            0,
            0,
            4,
            4,
            0,
        )
        .unwrap();
        assert_eq!(&picture.rgba[..4], &[254, 0, 0, 255]);
        // Crops offset the sample grid.
        let (y, cb, cr) = planes(235, 128, 128);
        let picture = theora_picture_from_planes(
            TheoraPlanes {
                y: &y,
                cb: &cb,
                cr: &cr,
            },
            1,
            1,
            2,
            2,
            0,
        )
        .unwrap();
        assert_eq!((picture.width, picture.height), (2, 2));
    }

    #[test]
    fn plane_and_crop_errors() {
        assert!(check_plane(0, 4, 4).is_err());
        assert!(check_plane(4, 4, 3).is_err());
        assert!(check_plane(1, 1, 256 * 1024 * 1024).is_err());
        let (y, cb, cr) = planes(16, 128, 128);
        assert!(theora_picture_from_planes(
            TheoraPlanes {
                y: &y,
                cb: &cb,
                cr: &cr
            },
            3,
            0,
            2,
            2,
            0
        )
        .is_err());
        let bad = TheoraPlaneBytes {
            bytes: vec![0; 4],
            width: 2,
            height: 2,
            stride: 2,
        };
        let narrow = TheoraPlaneBytes {
            bytes: vec![0; 2],
            width: 1,
            height: 2,
            stride: 1,
        };
        assert!(theora_picture_from_planes(
            TheoraPlanes {
                y: &y,
                cb: &bad,
                cr: &narrow
            },
            0,
            0,
            4,
            4,
            0
        )
        .is_err());
        let short = TheoraPlaneBytes {
            bytes: vec![16; 2],
            width: 4,
            height: 4,
            stride: 4,
        };
        assert!(theora_picture_from_planes(
            TheoraPlanes {
                y: &short,
                cb: &cb,
                cr: &cr
            },
            0,
            0,
            4,
            4,
            0
        )
        .is_err());
    }

    #[test]
    fn header_errors_do_not_require_streams() {
        assert!(TheoraDecoder::new(&[]).is_err());
        let fake = OggPacket {
            data: vec![0u8; 8],
            first: true,
            last: false,
            granule: 0,
            index: 0,
        };
        // Garbage fails header decode (no real stream needed).
        assert!(TheoraDecoder::new(&[fake.clone(), fake.clone(), fake]).is_err());
    }
}
