use std::fs::File;
use std::io::{BufRead, BufReader, Cursor, Seek};
use std::path::Path;

use image::{DynamicImage, ImageDecoder};

use crate::foundation::{Error, Result};

// ---------------------------------------------------------------------------
// ImageDecodeLimits
// ---------------------------------------------------------------------------

/// Resource limits applied before and during image decoding.
///
/// A zero value disables that individual limit. Use [`Self::unbounded`] only
/// for trusted input when the caller explicitly accepts unbounded decoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageDecodeLimits {
    pub max_input_bytes: u64,
    pub max_pixels: u64,
    pub max_width: u64,
    pub max_height: u64,
}

impl ImageDecodeLimits {
    pub const DEFAULT_MAX_INPUT_BYTES: u64 = 50 * 1024 * 1024;
    pub const DEFAULT_MAX_PIXELS: u64 = 50_000_000;
    pub const DEFAULT_MAX_WIDTH: u64 = 12_000;
    pub const DEFAULT_MAX_HEIGHT: u64 = 12_000;

    /// Construct a fully unbounded limit set for explicitly trusted input.
    pub const fn unbounded() -> Self {
        Self {
            max_input_bytes: 0,
            max_pixels: 0,
            max_width: 0,
            max_height: 0,
        }
    }

    pub(crate) fn check_input_bytes(
        self,
        actual: u64,
    ) -> std::result::Result<(), ImageDecodeLimitViolation> {
        if self.max_input_bytes > 0 && actual > self.max_input_bytes {
            return Err(ImageDecodeLimitViolation::InputBytes {
                actual,
                max: self.max_input_bytes,
            });
        }
        Ok(())
    }

    pub(crate) fn check_dimensions(
        self,
        width: u32,
        height: u32,
    ) -> std::result::Result<(), ImageDecodeLimitViolation> {
        let width = u64::from(width);
        let height = u64::from(height);
        let pixels = width.saturating_mul(height);

        if (self.max_width > 0 && width > self.max_width)
            || (self.max_height > 0 && height > self.max_height)
            || (self.max_pixels > 0 && pixels > self.max_pixels)
        {
            return Err(ImageDecodeLimitViolation::Dimensions {
                width,
                height,
                max_width: self.max_width,
                max_height: self.max_height,
                max_pixels: self.max_pixels,
            });
        }

        Ok(())
    }

    fn decoder_limits(self) -> image::Limits {
        if self == Self::unbounded() {
            return image::Limits::no_limits();
        }

        let mut limits = image::Limits::default();
        limits.max_image_width = nonzero_u32_limit(self.max_width);
        limits.max_image_height = nonzero_u32_limit(self.max_height);
        limits
    }
}

impl Default for ImageDecodeLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: Self::DEFAULT_MAX_INPUT_BYTES,
            max_pixels: Self::DEFAULT_MAX_PIXELS,
            max_width: Self::DEFAULT_MAX_WIDTH,
            max_height: Self::DEFAULT_MAX_HEIGHT,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImageDecodeLimitViolation {
    InputBytes {
        actual: u64,
        max: u64,
    },
    Dimensions {
        width: u64,
        height: u64,
        max_width: u64,
        max_height: u64,
        max_pixels: u64,
    },
}

fn nonzero_u32_limit(value: u64) -> Option<u32> {
    if value == 0 {
        None
    } else {
        Some(u32::try_from(value).unwrap_or(u32::MAX))
    }
}

// ---------------------------------------------------------------------------
// ImageFormat
// ---------------------------------------------------------------------------

/// Supported image formats for reading and writing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    Jpeg,
    Png,
    WebP,
    Gif,
    Bmp,
    Tiff,
    Avif,
    Ico,
}

impl ImageFormat {
    /// Detect format from a file extension string (without the leading dot).
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "png" => Some(Self::Png),
            "webp" => Some(Self::WebP),
            "gif" => Some(Self::Gif),
            "bmp" => Some(Self::Bmp),
            "tiff" | "tif" => Some(Self::Tiff),
            "avif" => Some(Self::Avif),
            "ico" => Some(Self::Ico),
            _ => None,
        }
    }

    /// Return the canonical file extension for this format.
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::WebP => "webp",
            Self::Gif => "gif",
            Self::Bmp => "bmp",
            Self::Tiff => "tiff",
            Self::Avif => "avif",
            Self::Ico => "ico",
        }
    }
}

impl From<ImageFormat> for image::ImageFormat {
    fn from(fmt: ImageFormat) -> Self {
        match fmt {
            ImageFormat::Jpeg => image::ImageFormat::Jpeg,
            ImageFormat::Png => image::ImageFormat::Png,
            ImageFormat::WebP => image::ImageFormat::WebP,
            ImageFormat::Gif => image::ImageFormat::Gif,
            ImageFormat::Bmp => image::ImageFormat::Bmp,
            ImageFormat::Tiff => image::ImageFormat::Tiff,
            ImageFormat::Avif => image::ImageFormat::Avif,
            ImageFormat::Ico => image::ImageFormat::Ico,
        }
    }
}

impl TryFrom<image::ImageFormat> for ImageFormat {
    type Error = Error;

    fn try_from(fmt: image::ImageFormat) -> Result<Self> {
        match fmt {
            image::ImageFormat::Jpeg => Ok(Self::Jpeg),
            image::ImageFormat::Png => Ok(Self::Png),
            image::ImageFormat::WebP => Ok(Self::WebP),
            image::ImageFormat::Gif => Ok(Self::Gif),
            image::ImageFormat::Bmp => Ok(Self::Bmp),
            image::ImageFormat::Tiff => Ok(Self::Tiff),
            image::ImageFormat::Avif => Ok(Self::Avif),
            image::ImageFormat::Ico => Ok(Self::Ico),
            other => Err(Error::Message(format!(
                "Unsupported image format: {other:?}"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// Rotation
// ---------------------------------------------------------------------------

/// Rotation angles for image transformation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotation {
    Deg90,
    Deg180,
    Deg270,
}

// ---------------------------------------------------------------------------
// ImageProcessor
// ---------------------------------------------------------------------------

const DEFAULT_JPEG_QUALITY: u8 = 85;

/// A chainable image processing builder.
///
/// # Example
///
/// ```rust,no_run
/// use foundry::imaging::{ImageProcessor, ImageFormat, Rotation};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let bytes = ImageProcessor::open("photo.jpg")?
///     .resize(800, 600)
///     .grayscale()
///     .quality(85)
///     .to_bytes(ImageFormat::Jpeg)?;
/// # Ok(())
/// # }
/// ```
pub struct ImageProcessor {
    image: DynamicImage,
    format: Option<ImageFormat>,
    jpeg_quality: Option<u8>,
}

impl ImageProcessor {
    // -- constructors -------------------------------------------------------

    /// Open an image from a file path with [`ImageDecodeLimits::default`].
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::open_with_limits(path, ImageDecodeLimits::default())
    }

    /// Open an image from a file path with custom decode limits.
    pub fn open_with_limits<P: AsRef<Path>>(path: P, limits: ImageDecodeLimits) -> Result<Self> {
        let path = path.as_ref();
        let format = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(ImageFormat::from_extension);
        let file = File::open(path)
            .map_err(|error| Error::message(format!("Failed to open image: {error}")))?;
        let input_bytes = file
            .metadata()
            .map_err(|error| Error::message(format!("Failed to inspect image: {error}")))?
            .len();
        limits
            .check_input_bytes(input_bytes)
            .map_err(image_decode_limit_error)?;

        let inner = BufReader::new(file);
        let reader = match image::ImageFormat::from_path(path) {
            Ok(format) => image::ImageReader::with_format(inner, format),
            Err(_) => image::ImageReader::new(inner),
        };
        let image = decode_reader(reader, limits, "Failed to open image")?;

        Ok(Self {
            image,
            format,
            jpeg_quality: None,
        })
    }

    /// Open an image without decode limits. Use only for explicitly trusted input.
    pub fn open_unbounded<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::open_with_limits(path, ImageDecodeLimits::unbounded())
    }

    /// Create an image processor from raw bytes with [`ImageDecodeLimits::default`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        Self::from_bytes_with_limits(bytes, ImageDecodeLimits::default())
    }

    /// Create an image processor from raw bytes with custom decode limits.
    pub fn from_bytes_with_limits(bytes: &[u8], limits: ImageDecodeLimits) -> Result<Self> {
        limits
            .check_input_bytes(bytes.len() as u64)
            .map_err(image_decode_limit_error)?;

        let reader = image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|error| Error::message(format!("Failed to guess image format: {error}")))?;

        let format = reader.format().and_then(|f| ImageFormat::try_from(f).ok());
        let image = decode_reader(reader, limits, "Failed to decode image")?;

        Ok(Self {
            image,
            format,
            jpeg_quality: None,
        })
    }

    /// Decode raw bytes without limits. Use only for explicitly trusted input.
    pub fn from_bytes_unbounded(bytes: &[u8]) -> Result<Self> {
        Self::from_bytes_with_limits(bytes, ImageDecodeLimits::unbounded())
    }

    /// Decode and process an image file on Tokio's blocking thread pool.
    pub async fn process_file<P, T, F>(path: P, process: F) -> Result<T>
    where
        P: AsRef<Path>,
        T: Send + 'static,
        F: FnOnce(Self) -> Result<T> + Send + 'static,
    {
        Self::process_file_with_limits(path, ImageDecodeLimits::default(), process).await
    }

    /// Decode and process an image file with custom limits on Tokio's blocking thread pool.
    pub async fn process_file_with_limits<P, T, F>(
        path: P,
        limits: ImageDecodeLimits,
        process: F,
    ) -> Result<T>
    where
        P: AsRef<Path>,
        T: Send + 'static,
        F: FnOnce(Self) -> Result<T> + Send + 'static,
    {
        let path = path.as_ref().to_path_buf();
        crate::support::run_blocking("image file processing", move || {
            process(Self::open_with_limits(path, limits)?)
        })
        .await
    }

    /// Decode and process owned image bytes on Tokio's blocking thread pool.
    pub async fn process_bytes<T, F>(bytes: Vec<u8>, process: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(Self) -> Result<T> + Send + 'static,
    {
        Self::process_bytes_with_limits(bytes, ImageDecodeLimits::default(), process).await
    }

    /// Decode and process owned bytes with custom limits on Tokio's blocking thread pool.
    pub async fn process_bytes_with_limits<T, F>(
        bytes: Vec<u8>,
        limits: ImageDecodeLimits,
        process: F,
    ) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(Self) -> Result<T> + Send + 'static,
    {
        crate::support::run_blocking("image byte processing", move || {
            process(Self::from_bytes_with_limits(&bytes, limits)?)
        })
        .await
    }

    // -- info ---------------------------------------------------------------

    /// Returns the image width in pixels.
    pub fn width(&self) -> u32 {
        self.image.width()
    }

    /// Returns the image height in pixels.
    pub fn height(&self) -> u32 {
        self.image.height()
    }

    /// Returns the detected image format, if known.
    pub fn format(&self) -> Option<ImageFormat> {
        self.format
    }

    // -- resize -------------------------------------------------------------

    /// Resize to exact dimensions, ignoring aspect ratio.
    pub fn resize(mut self, width: u32, height: u32) -> Self {
        self.image = self
            .image
            .resize_exact(width, height, image::imageops::FilterType::Lanczos3);
        self
    }

    /// Resize to fit within the given bounds while preserving aspect ratio.
    pub fn resize_to_fit(mut self, max_width: u32, max_height: u32) -> Self {
        self.image =
            self.image
                .resize(max_width, max_height, image::imageops::FilterType::Lanczos3);
        self
    }

    /// Resize and crop to fill the given dimensions while preserving aspect ratio.
    pub fn resize_to_fill(mut self, width: u32, height: u32) -> Self {
        self.image =
            self.image
                .resize_to_fill(width, height, image::imageops::FilterType::Lanczos3);
        self
    }

    // -- crop ---------------------------------------------------------------

    /// Crop a region from the image.
    pub fn crop(mut self, x: u32, y: u32, width: u32, height: u32) -> Self {
        self.image = self.image.crop_imm(x, y, width, height);
        self
    }

    // -- quality ------------------------------------------------------------

    /// Set JPEG encoding quality (1-100). Values are clamped.
    ///
    /// Encoding any non-JPEG format after setting quality returns an error.
    /// WebP output remains available through the built-in lossless encoder.
    pub fn quality(mut self, q: u8) -> Self {
        self.jpeg_quality = Some(q.clamp(1, 100));
        self
    }

    // -- effects ------------------------------------------------------------

    /// Apply a Gaussian blur with the given sigma.
    pub fn blur(mut self, sigma: f32) -> Self {
        self.image = self.image.blur(sigma);
        self
    }

    /// Convert the image to grayscale.
    pub fn grayscale(mut self) -> Self {
        self.image = self.image.grayscale();
        self
    }

    /// Rotate the image by the specified angle.
    pub fn rotate(mut self, rotation: Rotation) -> Self {
        self.image = match rotation {
            Rotation::Deg90 => self.image.rotate90(),
            Rotation::Deg180 => self.image.rotate180(),
            Rotation::Deg270 => self.image.rotate270(),
        };
        self
    }

    /// Flip the image horizontally.
    pub fn flip_horizontal(mut self) -> Self {
        self.image = self.image.fliph();
        self
    }

    /// Flip the image vertically.
    pub fn flip_vertical(mut self) -> Self {
        self.image = self.image.flipv();
        self
    }

    // -- adjustments --------------------------------------------------------

    /// Adjust brightness. Positive values brighten, negative darken.
    pub fn brightness(mut self, value: i32) -> Self {
        self.image = self.image.brighten(value);
        self
    }

    /// Adjust contrast. Positive values increase contrast, negative decrease.
    pub fn contrast(mut self, value: f32) -> Self {
        self.image = self.image.adjust_contrast(value);
        self
    }

    // -- output -------------------------------------------------------------

    /// Save the image to a file path, inferring format from the extension.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let path = path.as_ref();
        let format = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(ImageFormat::from_extension)
            .or(self.format)
            .ok_or_else(|| {
                Error::Message("Cannot determine output format from path".to_string())
            })?;

        self.write_to_path(path, format)
    }

    /// Save the image to a file path with an explicit format.
    pub fn save_as<P: AsRef<Path>>(&self, path: P, format: ImageFormat) -> Result<()> {
        self.write_to_path(path.as_ref(), format)
    }

    /// Encode the image to bytes in the given format.
    pub fn to_bytes(&self, format: ImageFormat) -> Result<Vec<u8>> {
        let mut cursor = Cursor::new(Vec::new());
        self.encode_to(&mut cursor, format)?;
        Ok(cursor.into_inner())
    }

    // -- internal -----------------------------------------------------------

    fn write_to_path(&self, path: &Path, format: ImageFormat) -> Result<()> {
        let bytes = self.to_bytes(format)?;
        std::fs::write(path, bytes)
            .map_err(|e| Error::Message(format!("Failed to write image: {e}")))?;
        Ok(())
    }

    fn encode_to<W: std::io::Write + std::io::Seek>(
        &self,
        writer: &mut W,
        format: ImageFormat,
    ) -> Result<()> {
        self.validate_quality_for_format(format)?;
        let img = &self.image;

        match format {
            ImageFormat::Jpeg => {
                let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
                    writer,
                    self.jpeg_quality.unwrap_or(DEFAULT_JPEG_QUALITY),
                );
                img.write_with_encoder(encoder)
                    .map_err(|e| Error::Message(format!("JPEG encode failed: {e}")))?;
            }
            ImageFormat::Png => {
                let encoder = image::codecs::png::PngEncoder::new(writer);
                img.write_with_encoder(encoder)
                    .map_err(|e| Error::Message(format!("PNG encode failed: {e}")))?;
            }
            ImageFormat::WebP => {
                // The image crate's built-in WebP encoder is lossless.
                img.write_to(writer, image::ImageFormat::WebP)
                    .map_err(|e| Error::Message(format!("WebP encode failed: {e}")))?;
            }
            ImageFormat::Gif => {
                img.write_to(writer, image::ImageFormat::Gif)
                    .map_err(|e| Error::Message(format!("GIF encode failed: {e}")))?;
            }
            ImageFormat::Bmp => {
                let encoder = image::codecs::bmp::BmpEncoder::new(writer);
                img.write_with_encoder(encoder)
                    .map_err(|e| Error::Message(format!("BMP encode failed: {e}")))?;
            }
            ImageFormat::Tiff => {
                let encoder = image::codecs::tiff::TiffEncoder::new(writer);
                img.write_with_encoder(encoder)
                    .map_err(|e| Error::Message(format!("TIFF encode failed: {e}")))?;
            }
            ImageFormat::Avif => {
                img.write_to(writer, image::ImageFormat::Avif)
                    .map_err(|e| Error::Message(format!("AVIF encode failed: {e}")))?;
            }
            ImageFormat::Ico => {
                img.write_to(writer, image::ImageFormat::Ico)
                    .map_err(|e| Error::Message(format!("ICO encode failed: {e}")))?;
            }
        }

        Ok(())
    }

    fn validate_quality_for_format(&self, format: ImageFormat) -> Result<()> {
        if self.jpeg_quality.is_none() || format == ImageFormat::Jpeg {
            return Ok(());
        }

        if format == ImageFormat::WebP {
            return Err(Error::message(
                "image quality is only supported for JPEG output; WebP output is lossless",
            ));
        }

        Err(Error::message(format!(
            "image quality is only supported for JPEG output, not .{}",
            format.extension()
        )))
    }
}

fn decode_reader<R>(
    mut reader: image::ImageReader<R>,
    limits: ImageDecodeLimits,
    error_context: &'static str,
) -> Result<DynamicImage>
where
    R: BufRead + Seek,
{
    let mut decoder_limits = limits.decoder_limits();
    reader.limits(decoder_limits.clone());

    let mut decoder = reader
        .into_decoder()
        .map_err(|error| image_reader_error(error_context, error))?;
    let (width, height) = decoder.dimensions();
    limits
        .check_dimensions(width, height)
        .map_err(image_decode_limit_error)?;

    decoder_limits
        .reserve(decoder.total_bytes())
        .map_err(|error| image_reader_error(error_context, error))?;
    decoder
        .set_limits(decoder_limits)
        .map_err(|error| image_reader_error(error_context, error))?;

    DynamicImage::from_decoder(decoder).map_err(|error| image_reader_error(error_context, error))
}

fn image_reader_error(error_context: &'static str, error: image::ImageError) -> Error {
    if matches!(&error, image::ImageError::Limits(_)) {
        return Error::message(format!("Image exceeds configured decode limits: {error}"));
    }

    Error::message(format!("{error_context}: {error}"))
}

fn image_decode_limit_error(violation: ImageDecodeLimitViolation) -> Error {
    match violation {
        ImageDecodeLimitViolation::InputBytes { actual, max } => Error::message(format!(
            "Image input exceeds decode limit ({actual} bytes; max {max})"
        )),
        ImageDecodeLimitViolation::Dimensions {
            width,
            height,
            max_width,
            max_height,
            max_pixels,
        } => Error::message(format!(
            "Image dimensions exceed decode limits ({width}x{height}; max width {max_width}, max height {max_height}, max pixels {max_pixels})"
        )),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use image::DynamicImage;

    fn test_image() -> ImageProcessor {
        let img = DynamicImage::new_rgba8(100, 100);
        ImageProcessor {
            image: img,
            format: Some(ImageFormat::Png),
            jpeg_quality: None,
        }
    }

    fn test_image_bytes(width: u32, height: u32, format: ImageFormat) -> Vec<u8> {
        ImageProcessor {
            image: DynamicImage::new_rgba8(width, height),
            format: Some(format),
            jpeg_quality: None,
        }
        .to_bytes(format)
        .unwrap()
    }

    #[test]
    fn resize_changes_dimensions() {
        let proc = test_image().resize(50, 30);
        assert_eq!(proc.width(), 50);
        assert_eq!(proc.height(), 30);
    }

    #[test]
    fn resize_to_fit_preserves_aspect_ratio() {
        let proc = test_image().resize_to_fit(50, 50);
        assert!(proc.width() <= 50);
        assert!(proc.height() <= 50);
    }

    #[test]
    fn resize_to_fill_matches_dimensions() {
        let proc = test_image().resize_to_fill(60, 40);
        assert_eq!(proc.width(), 60);
        assert_eq!(proc.height(), 40);
    }

    #[test]
    fn format_from_extension() {
        assert_eq!(ImageFormat::from_extension("jpg"), Some(ImageFormat::Jpeg));
        assert_eq!(ImageFormat::from_extension("jpeg"), Some(ImageFormat::Jpeg));
        assert_eq!(ImageFormat::from_extension("PNG"), Some(ImageFormat::Png));
        assert_eq!(ImageFormat::from_extension("webp"), Some(ImageFormat::WebP));
        assert_eq!(ImageFormat::from_extension("gif"), Some(ImageFormat::Gif));
        assert_eq!(ImageFormat::from_extension("bmp"), Some(ImageFormat::Bmp));
        assert_eq!(ImageFormat::from_extension("tiff"), Some(ImageFormat::Tiff));
        assert_eq!(ImageFormat::from_extension("tif"), Some(ImageFormat::Tiff));
        assert_eq!(ImageFormat::from_extension("avif"), Some(ImageFormat::Avif));
        assert_eq!(ImageFormat::from_extension("ico"), Some(ImageFormat::Ico));
        assert_eq!(ImageFormat::from_extension("xyz"), None);
    }

    #[test]
    fn format_extension_roundtrip() {
        let formats = [
            ImageFormat::Jpeg,
            ImageFormat::Png,
            ImageFormat::WebP,
            ImageFormat::Gif,
            ImageFormat::Bmp,
            ImageFormat::Tiff,
            ImageFormat::Avif,
            ImageFormat::Ico,
        ];
        for fmt in formats {
            let ext = fmt.extension();
            assert_eq!(ImageFormat::from_extension(ext), Some(fmt));
        }
    }

    #[test]
    fn quality_clamping() {
        let proc = test_image().quality(0);
        assert_eq!(proc.jpeg_quality, Some(1));

        let proc = test_image().quality(150);
        assert_eq!(proc.jpeg_quality, Some(100));

        let proc = test_image().quality(75);
        assert_eq!(proc.jpeg_quality, Some(75));
    }

    #[test]
    fn webp_is_lossless_and_rejects_explicit_quality() {
        let bytes = test_image().to_bytes(ImageFormat::WebP).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WEBP");

        let error = test_image()
            .quality(80)
            .to_bytes(ImageFormat::WebP)
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "image quality is only supported for JPEG output; WebP output is lossless"
        );
    }

    #[test]
    fn explicit_quality_is_rejected_for_other_non_jpeg_formats() {
        let error = test_image()
            .quality(80)
            .to_bytes(ImageFormat::Png)
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "image quality is only supported for JPEG output, not .png"
        );
    }

    #[test]
    fn to_bytes_produces_valid_png() {
        let proc = test_image();
        let bytes = proc.to_bytes(ImageFormat::Png).unwrap();
        assert!(!bytes.is_empty());
        // PNG magic bytes
        assert_eq!(&bytes[..4], &[0x89, 0x50, 0x4E, 0x47]);
    }

    #[test]
    fn to_bytes_produces_valid_jpeg() {
        let proc = test_image();
        let bytes = proc.to_bytes(ImageFormat::Jpeg).unwrap();
        assert!(!bytes.is_empty());
        // JPEG magic bytes
        assert_eq!(&bytes[..2], &[0xFF, 0xD8]);
    }

    #[test]
    fn grayscale_preserves_dimensions() {
        let proc = test_image().grayscale();
        assert_eq!(proc.width(), 100);
        assert_eq!(proc.height(), 100);
    }

    #[test]
    fn rotate_90_swaps_dimensions() {
        let img = DynamicImage::new_rgba8(100, 50);
        let proc = ImageProcessor {
            image: img,
            format: Some(ImageFormat::Png),
            jpeg_quality: None,
        };
        let proc = proc.rotate(Rotation::Deg90);
        assert_eq!(proc.width(), 50);
        assert_eq!(proc.height(), 100);
    }

    #[test]
    fn crop_changes_dimensions() {
        let proc = test_image().crop(10, 10, 50, 30);
        assert_eq!(proc.width(), 50);
        assert_eq!(proc.height(), 30);
    }

    #[test]
    fn from_bytes_roundtrip() {
        let original = test_image();
        let bytes = original.to_bytes(ImageFormat::Png).unwrap();
        let loaded = ImageProcessor::from_bytes(&bytes).unwrap();
        assert_eq!(loaded.width(), 100);
        assert_eq!(loaded.height(), 100);
        assert_eq!(loaded.format(), Some(ImageFormat::Png));
    }

    #[test]
    fn bounded_bytes_reject_input_size_and_pixel_limits() {
        let bytes = test_image_bytes(20, 10, ImageFormat::Png);
        let input_error = ImageProcessor::from_bytes_with_limits(
            &bytes,
            ImageDecodeLimits {
                max_input_bytes: bytes.len() as u64 - 1,
                ..ImageDecodeLimits::default()
            },
        )
        .err()
        .unwrap();
        assert!(input_error
            .to_string()
            .contains("Image input exceeds decode limit"));

        let pixel_error = ImageProcessor::from_bytes_with_limits(
            &bytes,
            ImageDecodeLimits {
                max_input_bytes: 0,
                max_pixels: 199,
                max_width: 0,
                max_height: 0,
            },
        )
        .err()
        .unwrap();
        assert!(pixel_error
            .to_string()
            .contains("Image dimensions exceed decode limits (20x10"));
    }

    #[test]
    fn public_defaults_are_bounded_and_unbounded_constructor_is_explicit() {
        let bytes = test_image_bytes(
            ImageDecodeLimits::DEFAULT_MAX_WIDTH as u32 + 1,
            1,
            ImageFormat::Png,
        );

        let error = ImageProcessor::from_bytes(&bytes).err().unwrap();
        assert!(error.to_string().contains("configured decode limits"));

        let trusted = ImageProcessor::from_bytes_unbounded(&bytes).unwrap();
        assert_eq!(trusted.width(), 12_001);
    }

    #[test]
    fn file_constructors_share_bounded_and_explicit_unbounded_semantics() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("wide.png");
        std::fs::write(
            &path,
            test_image_bytes(
                ImageDecodeLimits::DEFAULT_MAX_WIDTH as u32 + 1,
                1,
                ImageFormat::Png,
            ),
        )
        .unwrap();

        let error = ImageProcessor::open(&path).err().unwrap();
        assert!(error.to_string().contains("configured decode limits"));

        let trusted = ImageProcessor::open_unbounded(&path).unwrap();
        assert_eq!(trusted.width(), 12_001);
    }

    #[tokio::test]
    async fn process_file_runs_the_full_callback_on_the_blocking_pool() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("input.png");
        std::fs::write(&path, test_image_bytes(40, 20, ImageFormat::Png)).unwrap();

        let output = ImageProcessor::process_file(path, |image| {
            image.resize(10, 5).to_bytes(ImageFormat::Jpeg)
        })
        .await
        .unwrap();

        assert_eq!(&output[..2], &[0xFF, 0xD8]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn process_bytes_keeps_the_async_runtime_responsive() {
        let bytes = test_image_bytes(40, 20, ImageFormat::Png);
        let started = Instant::now();
        let task = tokio::spawn(ImageProcessor::process_bytes(bytes, |image| {
            std::thread::sleep(Duration::from_millis(300));
            Ok(image.width())
        }));

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(started.elapsed() < Duration::from_millis(150));
        assert_eq!(task.await.unwrap().unwrap(), 40);
    }

    #[test]
    fn image_format_conversion() {
        let foundry_fmt = ImageFormat::Jpeg;
        let img_fmt: image::ImageFormat = foundry_fmt.into();
        assert_eq!(img_fmt, image::ImageFormat::Jpeg);

        let back: ImageFormat = img_fmt.try_into().unwrap();
        assert_eq!(back, ImageFormat::Jpeg);
    }
}
