use std::io::Cursor;
use std::path::Path;

use image::DynamicImage;

use crate::foundation::{Error, Result};

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
    quality: u8,
}

impl ImageProcessor {
    // -- constructors -------------------------------------------------------

    /// Open an image from a file path.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let format = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(ImageFormat::from_extension);

        let img =
            image::open(path).map_err(|e| Error::Message(format!("Failed to open image: {e}")))?;

        Ok(Self {
            image: img,
            format,
            quality: 85,
        })
    }

    /// Create an image processor from raw bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let reader = image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|e| Error::Message(format!("Failed to guess image format: {e}")))?;

        let format = reader.format().and_then(|f| ImageFormat::try_from(f).ok());

        let img = reader
            .decode()
            .map_err(|e| Error::Message(format!("Failed to decode image: {e}")))?;

        Ok(Self {
            image: img,
            format,
            quality: 85,
        })
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

    /// Set the JPEG/WebP encoding quality (1-100). Values are clamped.
    pub fn quality(mut self, q: u8) -> Self {
        self.quality = q.clamp(1, 100);
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
        let img = &self.image;

        match format {
            ImageFormat::Jpeg => {
                let encoder =
                    image::codecs::jpeg::JpegEncoder::new_with_quality(writer, self.quality);
                img.write_with_encoder(encoder)
                    .map_err(|e| Error::Message(format!("JPEG encode failed: {e}")))?;
            }
            ImageFormat::Png => {
                let encoder = image::codecs::png::PngEncoder::new(writer);
                img.write_with_encoder(encoder)
                    .map_err(|e| Error::Message(format!("PNG encode failed: {e}")))?;
            }
            ImageFormat::WebP => {
                // WebP encoder in the image crate does not support quality setting directly.
                // Use the standard save path.
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
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use image::DynamicImage;

    fn test_image() -> ImageProcessor {
        let img = DynamicImage::new_rgba8(100, 100);
        ImageProcessor {
            image: img,
            format: Some(ImageFormat::Png),
            quality: 85,
        }
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
        assert_eq!(proc.quality, 1);

        let proc = test_image().quality(150);
        assert_eq!(proc.quality, 100);

        let proc = test_image().quality(75);
        assert_eq!(proc.quality, 75);
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
            quality: 85,
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
    fn image_format_conversion() {
        let foundry_fmt = ImageFormat::Jpeg;
        let img_fmt: image::ImageFormat = foundry_fmt.into();
        assert_eq!(img_fmt, image::ImageFormat::Jpeg);

        let back: ImageFormat = img_fmt.try_into().unwrap();
        assert_eq!(back, ImageFormat::Jpeg);
    }
}
