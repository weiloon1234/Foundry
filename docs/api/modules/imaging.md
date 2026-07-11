# imaging

Image processing pipeline (resize, crop, rotate, format conversion)

[Back to index](../index.md)

## foundry::imaging

```rust
enum ImageFormat { Jpeg, Png, WebP, Gif, Bmp, Tiff, Avif, Ico }
  fn from_extension(ext: &str) -> Option<Self>
  fn extension(&self) -> &'static str
enum Rotation { Deg90, Deg180, Deg270 }
struct ImageDecodeLimits
  const DEFAULT_MAX_INPUT_BYTES: u64
  const DEFAULT_MAX_PIXELS: u64 = 50_000_000
  const DEFAULT_MAX_WIDTH: u64 = 12_000
  const DEFAULT_MAX_HEIGHT: u64 = 12_000
  const fn unbounded() -> Self
struct ImageProcessor
  fn open<P: AsRef<Path>>(path: P) -> Result<Self>
  fn open_with_limits<P: AsRef<Path>>( path: P, limits: ImageDecodeLimits, ) -> Result<Self>
  fn open_unbounded<P: AsRef<Path>>(path: P) -> Result<Self>
  fn from_bytes(bytes: &[u8]) -> Result<Self>
  fn from_bytes_with_limits( bytes: &[u8], limits: ImageDecodeLimits, ) -> Result<Self>
  fn from_bytes_unbounded(bytes: &[u8]) -> Result<Self>
  async fn process_file<P, T, F>(path: P, process: F) -> Result<T>
  async fn process_file_with_limits<P, T, F>( path: P, limits: ImageDecodeLimits, process: F, ) -> Result<T>
  async fn process_bytes<T, F>(bytes: Vec<u8>, process: F) -> Result<T>
  async fn process_bytes_with_limits<T, F>( bytes: Vec<u8>, limits: ImageDecodeLimits, process: F, ) -> Result<T>
  fn width(&self) -> u32
  fn height(&self) -> u32
  fn format(&self) -> Option<ImageFormat>
  fn resize(self, width: u32, height: u32) -> Self
  fn resize_to_fit(self, max_width: u32, max_height: u32) -> Self
  fn resize_to_fill(self, width: u32, height: u32) -> Self
  fn crop(self, x: u32, y: u32, width: u32, height: u32) -> Self
  fn quality(self, q: u8) -> Self
  fn blur(self, sigma: f32) -> Self
  fn grayscale(self) -> Self
  fn rotate(self, rotation: Rotation) -> Self
  fn flip_horizontal(self) -> Self
  fn flip_vertical(self) -> Self
  fn brightness(self, value: i32) -> Self
  fn contrast(self, value: f32) -> Self
  fn save<P: AsRef<Path>>(&self, path: P) -> Result<()>
  fn save_as<P: AsRef<Path>>( &self, path: P, format: ImageFormat, ) -> Result<()>
  fn to_bytes(&self, format: ImageFormat) -> Result<Vec<u8>>
```
