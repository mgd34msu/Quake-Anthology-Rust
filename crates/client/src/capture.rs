//! Screenshot and levelshot capture over owned frame readbacks.
//!
//! Donor provenance: `src/capture/index.ts` (Q3 screenshot and levelshot
//! capture, id Software, GPL-2.0-or-later). Same frame validation,
//! levelshot downsampling (`R_LevelShot`: a 512x384 grid averaged twelve
//! samples into each 128x128 texel), screenshot filename sequencing
//! (`shot0000`–`shot9999`), and path policy. The TGA/PNG/JPEG encoders
//! live in the donor's `src/formats/images/*`, ported as the
//! `qa-content` image codecs; encoding arrives as injected callbacks so
//! tests and embedders can override it, with [`encode_tga_image`],
//! [`encode_png_image`], and [`encode_jpeg_image`] as the real defaults;
//! file sequencing and the levelshot sampler are fully owned here.

use std::path::{Component, Path, PathBuf};

use qa_content::images::{encode_jpeg, encode_png, encode_tga, ImageLevel, JpegImage};

use crate::ClientError;

/// Screenshot image format (donor `ScreenshotFormat`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenshotFormat {
    /// Truevision TGA (`screenshot`).
    Tga,
    /// PNG (`screenshotPNG`).
    Png,
    /// JPEG (`screenshotJPEG`).
    Jpg,
}

impl ScreenshotFormat {
    /// File extension.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Tga => "tga",
            Self::Png => "png",
            Self::Jpg => "jpg",
        }
    }

    /// Parse a donor format name.
    pub fn parse(text: &str) -> Result<Self, ClientError> {
        match text {
            "tga" => Ok(Self::Tga),
            "png" => Ok(Self::Png),
            "jpg" => Ok(Self::Jpg),
            _ => Err(ClientError::BadCapture(format!("Unknown screenshot format {text:?}"))),
        }
    }
}

/// Owned RGBA frame (donor `ImageLevel`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major RGBA bytes.
    pub pixels: Vec<u8>,
}

/// Validate renderer readback dimensions (donor `checkFrame`).
pub fn check_frame(image: &RgbaImage) -> Result<(), ClientError> {
    if image.width < 1 || image.height < 1 || image.pixels.len() != image.width as usize * image.height as usize * 4 {
        return Err(ClientError::BadCaptureFrame);
    }
    Ok(())
}

/// Lossless image encoder callback.
pub type StillEncoder = dyn Fn(&RgbaImage) -> Result<Vec<u8>, ClientError>;
/// JPEG encoder callback (quality `0..=100`).
pub type JpegEncoder = dyn Fn(&RgbaImage, u8) -> Result<Vec<u8>, ClientError>;

/// Default screenshot encoders over the `qa-content` image codecs.
///
/// These match [`StillEncoder`]/[`JpegEncoder`], so callers that want the
/// real codecs write `ScreenshotEncoders { tga: &encode_tga_image, .. }`.
pub fn encode_tga_image(image: &RgbaImage) -> Result<Vec<u8>, ClientError> {
    check_frame(image)?;
    if image.width > 65535 || image.height > 65535 {
        return Err(ClientError::BadCapture("TGA frame exceeds 65535 pixels".to_string()));
    }
    Ok(encode_tga(&ImageLevel {
        width: image.width,
        height: image.height,
        pixels: image.pixels.clone(),
    }))
}

/// PNG encoder over the `qa-content` image codecs.
pub fn encode_png_image(image: &RgbaImage) -> Result<Vec<u8>, ClientError> {
    check_frame(image)?;
    encode_png(image.width, image.height, &image.pixels).map_err(|error| ClientError::BadCapture(error.to_string()))
}

/// JPEG encoder over the `qa-content` image codecs.
pub fn encode_jpeg_image(image: &RgbaImage, quality: u8) -> Result<Vec<u8>, ClientError> {
    check_frame(image)?;
    if image.width > 65500 || image.height > 65500 {
        return Err(ClientError::BadCapture("JPEG frame exceeds 65500 pixels".to_string()));
    }
    Ok(encode_jpeg(
        &JpegImage {
            width: image.width,
            height: image.height,
            pixels: image.pixels.clone(),
        },
        quality,
    ))
}

/// Image encoders (injectable; the real codecs are the defaults above).
pub struct ScreenshotEncoders<'a> {
    /// TGA encoder.
    pub tga: &'a StillEncoder,
    /// PNG encoder.
    pub png: &'a StillEncoder,
    /// JPEG encoder.
    pub jpg: &'a JpegEncoder,
}

/// Default encoder triple: TGA, PNG, JPEG.
pub type DefaultEncoders = (
    fn(&RgbaImage) -> Result<Vec<u8>, ClientError>,
    fn(&RgbaImage) -> Result<Vec<u8>, ClientError>,
    fn(&RgbaImage, u8) -> Result<Vec<u8>, ClientError>,
);

/// Default encoders bound to the real `qa-content` image codecs.
pub fn default_encoders() -> DefaultEncoders {
    (encode_tga_image, encode_png_image, encode_jpeg_image)
}

/// Encode a screenshot (donor `encodeScreenshot`).
pub fn encode_screenshot(
    image: &RgbaImage,
    format: ScreenshotFormat,
    quality: u8,
    encoders: &ScreenshotEncoders<'_>,
) -> Result<Vec<u8>, ClientError> {
    check_frame(image)?;
    match format {
        ScreenshotFormat::Tga => (encoders.tga)(image),
        ScreenshotFormat::Png => (encoders.png)(image),
        ScreenshotFormat::Jpg => (encoders.jpg)(image, quality),
    }
}

/// Levelshot size in pixels.
pub const LEVELSHOT_SIZE: u32 = 128;

/// Downsample a frame to a 128x128 levelshot (donor `makeLevelshot`).
pub fn make_levelshot(image: &RgbaImage, gamma: Option<&[u8; 256]>) -> Result<RgbaImage, ClientError> {
    check_frame(image)?;
    let mut pixels = vec![0u8; LEVELSHOT_SIZE as usize * LEVELSHOT_SIZE as usize * 4];
    let x_scale = image.width as f32 / 512.0;
    let y_scale = image.height as f32 / 384.0;
    for y in 0..LEVELSHOT_SIZE {
        for x in 0..LEVELSHOT_SIZE {
            let mut red = 0u32;
            let mut green = 0u32;
            let mut blue = 0u32;
            for yy in 0..3 {
                for xx in 0..4 {
                    let sx = (((x * 4 + xx) as f32 * x_scale) as u32).min(image.width - 1);
                    let sy = (((y * 3 + yy) as f32 * y_scale) as u32).min(image.height - 1);
                    let offset = (sy * image.width + sx) as usize * 4;
                    red += u32::from(image.pixels[offset]);
                    green += u32::from(image.pixels[offset + 1]);
                    blue += u32::from(image.pixels[offset + 2]);
                }
            }
            let offset = (y * LEVELSHOT_SIZE + x) as usize * 4;
            let map = |sum: u32| {
                let average = sum / 12;
                #[allow(clippy::cast_possible_truncation)]
                gamma.map_or(average as u8, |table| table[average as usize])
            };
            pixels[offset] = map(red);
            pixels[offset + 1] = map(green);
            pixels[offset + 2] = map(blue);
            pixels[offset + 3] = 255;
        }
    }
    Ok(RgbaImage {
        width: LEVELSHOT_SIZE,
        height: LEVELSHOT_SIZE,
        pixels,
    })
}

/// First free screenshot slot search bound (`shot0000`–`shot9999`).
pub const MAX_SCREENSHOT_INDEX: u32 = 10_000;

/// Sequenced screenshot file name (`shot0042.png`).
#[must_use]
pub fn screenshot_file_name(index: u32, format: ScreenshotFormat) -> String {
    format!("shot{:04}.{}", index, format.extension())
}

/// Named screenshot path under the capture root.
#[must_use]
pub fn named_screenshot_path(name: &str, format: ScreenshotFormat) -> String {
    format!("screenshots/{}.{}", name, format.extension())
}

/// Levelshot path for a map under the capture root.
#[must_use]
pub fn levelshot_path(map_name: &str) -> String {
    let map = map_name.strip_prefix("maps/").unwrap_or(map_name);
    let map = map.strip_suffix(".bsp").unwrap_or(map);
    format!("levelshots/{map}.tga")
}

/// Resolve `name` under `root`, rejecting escapes (mirrors the settings
/// path policy; `qa-client` cannot depend on `qa-app`).
pub fn capture_path(root: &Path, name: &str) -> Result<PathBuf, ClientError> {
    if name.is_empty() || name.contains('\0') {
        return Err(ClientError::BadCapturePath(name.to_string()));
    }
    let mut cleaned = PathBuf::new();
    for component in Path::new(name).components() {
        match component {
            Component::Normal(part) => cleaned.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ClientError::BadCapturePath(name.to_string()));
            }
        }
    }
    if cleaned.as_os_str().is_empty() {
        return Err(ClientError::BadCapturePath(name.to_string()));
    }
    Ok(root.join(cleaned))
}

/// Frame readback owned by the seat renderer (donor `FrameReadback`).
pub trait FrameReadback {
    /// Read one RGBA frame.
    fn read_rgba(&mut self) -> Result<RgbaImage, ClientError>;
}

/// Capture result (donor `CaptureResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureResult {
    /// Written path.
    pub path: String,
    /// Image width.
    pub width: u32,
    /// Image height.
    pub height: u32,
    /// Encoded byte length.
    pub byte_length: usize,
}

/// Owned screenshot/levelshot writer (donor `FrameCapture`).
///
/// Writes serialize through `&mut self`, matching the donor's chained
/// write queue without threads.
pub struct FrameCapture<'a> {
    root: PathBuf,
    readback: Box<dyn FrameReadback + 'a>,
    encoders: ScreenshotEncoders<'a>,
}

impl<'a> FrameCapture<'a> {
    /// Open a capture writer at `root` with injected readback and encoders.
    #[must_use]
    pub fn new(root: PathBuf, readback: Box<dyn FrameReadback + 'a>, encoders: ScreenshotEncoders<'a>) -> Self {
        Self {
            root,
            readback,
            encoders,
        }
    }

    /// Capture a screenshot: named files overwrite, sequenced files claim
    /// the first free `shotNNNN` slot.
    pub fn screenshot(
        &mut self,
        format: ScreenshotFormat,
        name: Option<&str>,
        quality: u8,
    ) -> Result<CaptureResult, ClientError> {
        let image = self.readback.read_rgba()?;
        let bytes = encode_screenshot(&image, format, quality, &self.encoders)?;
        if let Some(name) = name {
            let path = capture_path(&self.root, &named_screenshot_path(name, format))?;
            write_bytes(&path, &bytes)?;
            return Ok(CaptureResult {
                path: path.to_string_lossy().into_owned(),
                width: image.width,
                height: image.height,
                byte_length: bytes.len(),
            });
        }
        for index in 0..MAX_SCREENSHOT_INDEX {
            let path = capture_path(
                &self.root,
                &format!("screenshots/{}", screenshot_file_name(index, format)),
            )?;
            if try_claim(&path, &bytes)? {
                return Ok(CaptureResult {
                    path: path.to_string_lossy().into_owned(),
                    width: image.width,
                    height: image.height,
                    byte_length: bytes.len(),
                });
            }
        }
        Err(ClientError::NoFreeScreenshot)
    }

    /// Capture a levelshot for `map_name` (always TGA).
    pub fn levelshot(&mut self, map_name: &str, gamma: Option<&[u8; 256]>) -> Result<CaptureResult, ClientError> {
        let image = make_levelshot(&self.readback.read_rgba()?, gamma)?;
        let bytes = (self.encoders.tga)(&image)?;
        let path = capture_path(&self.root, &levelshot_path(map_name))?;
        write_bytes(&path, &bytes)?;
        Ok(CaptureResult {
            path: path.to_string_lossy().into_owned(),
            width: image.width,
            height: image.height,
            byte_length: bytes.len(),
        })
    }
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), ClientError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|error| ClientError::BadCaptureWrite(error.to_string()))?;
        }
    }
    std::fs::write(path, bytes).map_err(|error| ClientError::BadCaptureWrite(error.to_string()))
}

/// Claim a sequenced slot; returns false when the file already exists.
fn try_claim(path: &Path, bytes: &[u8]) -> Result<bool, ClientError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|error| ClientError::BadCaptureWrite(error.to_string()))?;
        }
    }
    match std::fs::OpenOptions::new().create_new(true).write(true).open(path) {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(bytes)
                .map_err(|error| ClientError::BadCaptureWrite(error.to_string()))?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(ClientError::BadCaptureWrite(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        image: RgbaImage,
    }

    impl FrameReadback for Fixture {
        fn read_rgba(&mut self) -> Result<RgbaImage, ClientError> {
            Ok(self.image.clone())
        }
    }

    fn gradient(width: u32, height: u32) -> RgbaImage {
        let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height {
            for x in 0..width {
                pixels.extend([x as u8, y as u8, 128, 255]);
            }
        }
        RgbaImage { width, height, pixels }
    }

    type StillEncoder = fn(&RgbaImage) -> Result<Vec<u8>, ClientError>;
    type QualityEncoder = fn(&RgbaImage, u8) -> Result<Vec<u8>, ClientError>;

    fn encoders() -> (StillEncoder, StillEncoder, QualityEncoder) {
        (
            |image: &RgbaImage| Ok(vec![b'T', image.width as u8, image.height as u8]),
            |image: &RgbaImage| Ok(vec![b'P', image.width as u8]),
            |_image: &RgbaImage, quality: u8| Ok(vec![b'J', quality]),
        )
    }

    #[test]
    fn downsamples_levelshots_with_gamma() {
        let image = gradient(512, 384);
        let level = make_levelshot(&image, None).unwrap();
        assert_eq!((level.width, level.height), (128, 128));
        assert_eq!(level.pixels[3], 255);
        let identity: Vec<u8> = (0..=255).collect();
        let identity: [u8; 256] = identity.try_into().unwrap();
        let mapped = make_levelshot(&image, Some(&identity)).unwrap();
        assert_eq!(mapped.pixels, level.pixels);
        assert!(check_frame(&RgbaImage {
            width: 0,
            height: 4,
            pixels: Vec::new()
        })
        .is_err());
    }

    #[test]
    fn sequences_and_names_screenshots() {
        assert_eq!(screenshot_file_name(42, ScreenshotFormat::Png), "shot0042.png");
        assert_eq!(levelshot_path("maps/q3dm1.bsp"), "levelshots/q3dm1.tga");
        assert_eq!(ScreenshotFormat::parse("jpg").unwrap(), ScreenshotFormat::Jpg);
        assert!(ScreenshotFormat::parse("gif").is_err());
        let root = std::env::temp_dir().join(format!("qa-capture-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (tga, png, jpg) = encoders();
        let mut capture = FrameCapture::new(
            root.clone(),
            Box::new(Fixture { image: gradient(8, 8) }),
            ScreenshotEncoders {
                tga: &tga,
                png: &png,
                jpg: &jpg,
            },
        );
        let first = capture.screenshot(ScreenshotFormat::Png, None, 90).unwrap();
        assert!(first.path.ends_with("shot0000.png"));
        let second = capture.screenshot(ScreenshotFormat::Png, None, 90).unwrap();
        assert!(second.path.ends_with("shot0001.png"));
        let named = capture.screenshot(ScreenshotFormat::Jpg, Some("custom"), 80).unwrap();
        assert!(named.path.ends_with("custom.jpg"));
        let level = capture.levelshot("q3dm1", None).unwrap();
        assert!(level.path.ends_with("q3dm1.tga"));
        assert_eq!((level.width, level.height), (128, 128));
        assert!(capture_path(&root, "../escape").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn default_encoders_emit_real_codecs() {
        let image = gradient(8, 8);
        let (tga, png, jpg) = default_encoders();
        let tga_bytes = tga(&image).unwrap();
        let back = qa_content::images::decode_tga(&tga_bytes, "<test>").unwrap();
        assert_eq!((back.width, back.height), (8, 8));
        assert_eq!(back.pixels, image.pixels);
        let png_bytes = png(&image).unwrap();
        assert_eq!(&png_bytes[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        let back = qa_content::images::decode_png(&png_bytes, "<test>").unwrap();
        assert_eq!((back.width, back.height), (8, 8));
        assert_eq!(back.pixels, image.pixels);
        let jpg_bytes = jpg(&image, 90).unwrap();
        assert_eq!(&jpg_bytes[..2], &[0xff, 0xd8]);
        assert_eq!(&jpg_bytes[jpg_bytes.len() - 2..], &[0xff, 0xd9]);
        let back = qa_content::images::decode_jpeg(&jpg_bytes, "<test>").unwrap();
        assert_eq!((back.width, back.height), (8, 8));
        let bad = RgbaImage {
            width: 0,
            height: 4,
            pixels: Vec::new(),
        };
        assert!(tga(&bad).is_err());
        assert!(png(&bad).is_err());
        assert!(jpg(&bad, 90).is_err());
    }
}
