use smart_leds_trait::RGB8;

#[derive(Clone, Copy, Debug, Eq, PartialEq, defmt::Format)]
pub struct BitmapStorageMetadata {
    pub width: usize,
    pub height: usize,
}

impl BitmapStorageMetadata {
    pub const fn pixel_count(self) -> usize {
        self.width * self.height
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Bitmap<'a> {
    metadata: BitmapStorageMetadata,
    pixels: &'a [RGB8],
}

impl<'a> Bitmap<'a> {
    pub fn new(metadata: BitmapStorageMetadata, pixels: &'a [RGB8]) -> Self {
        assert!(metadata.pixel_count() == pixels.len());
        Self { metadata, pixels }
    }

    pub fn metadata(&self) -> BitmapStorageMetadata {
        self.metadata
    }

    pub fn width(&self) -> usize {
        self.metadata.width
    }

    pub fn height(&self) -> usize {
        self.metadata.height
    }

    pub fn pixels(&self) -> &'a [RGB8] {
        self.pixels
    }

    pub fn scale_into(
        &self,
        target_width: usize,
        target_height: usize,
        target_pixels: &mut [RGB8],
    ) -> Result<(), BitmapError> {
        let expected = target_width.saturating_mul(target_height);
        if target_pixels.len() != expected {
            return Err(BitmapError::UnexpectedPixelCount {
                expected,
                actual: target_pixels.len(),
            });
        }

        if target_width == 0 || target_height == 0 {
            return Ok(());
        }

        let source_width = self.width();
        let source_height = self.height();

        for target_y in 0..target_height {
            let source_y = target_y * source_height / target_height;

            for target_x in 0..target_width {
                let source_x = target_x * source_width / target_width;
                let source_index = source_y * source_width + source_x;
                let target_index = target_y * target_width + target_x;

                target_pixels[target_index] = self.pixels[source_index];
            }
        }

        Ok(())
    }
}

pub struct BitmapMut<'a> {
    metadata: BitmapStorageMetadata,
    pixels: &'a mut [RGB8],
}

impl<'a> BitmapMut<'a> {
    pub fn new(metadata: BitmapStorageMetadata, pixels: &'a mut [RGB8]) -> Self {
        assert!(metadata.pixel_count() == pixels.len());
        Self { metadata, pixels }
    }

    /// Like `new`, but requires only that `pixels.len() >= metadata.pixel_count()`.
    /// Only the first `metadata.pixel_count()` elements of `pixels` are stored.
    /// Use this when the backing buffer is larger than the image being loaded
    /// (e.g. a max-capacity buffer that holds both Cartesian and polar images).
    pub fn new_with_capacity(metadata: BitmapStorageMetadata, pixels: &'a mut [RGB8]) -> Self {
        let pixel_count = metadata.pixel_count();
        assert!(pixels.len() >= pixel_count);
        Self {
            metadata,
            pixels: &mut pixels[..pixel_count],
        }
    }

    pub fn as_bitmap(&self) -> Bitmap<'_> {
        Bitmap::new(self.metadata, self.pixels)
    }

    pub fn metadata(&self) -> BitmapStorageMetadata {
        self.metadata
    }

    pub fn width(&self) -> usize {
        self.metadata.width
    }

    pub fn height(&self) -> usize {
        self.metadata.height
    }

    pub fn pixels(&self) -> &[RGB8] {
        self.pixels
    }

    pub fn pixels_mut(&mut self) -> &mut [RGB8] {
        self.pixels
    }

    pub fn fill(&mut self, color: RGB8) {
        for pixel in self.pixels.iter_mut() {
            *pixel = color;
        }
    }

    pub fn clear(&mut self) {
        self.fill(RGB8::default());
    }
}

#[derive(Debug, defmt::Format)]
pub enum BitmapError {
    InvalidIndex { index: usize, bitmap_count: usize },
    NotWritable { index: usize },
    UnexpectedPixelCount { expected: usize, actual: usize },
}

pub trait BitmapStorage {
    fn metadata(&self) -> BitmapStorageMetadata;

    fn bitmap_count(&self) -> usize;

    fn bitmap(&self, index: usize) -> Result<Bitmap<'_>, BitmapError>;

    fn bitmap_mut(&mut self, index: usize) -> Result<BitmapMut<'_>, BitmapError>;

    /// Switch the active bitmap to the static built-in image.
    /// Implementations that do not support swapping may ignore this.
    fn activate_builtin(&mut self) {}

    /// Switch the active bitmap to the decoded download buffer.
    /// Implementations that do not support swapping may ignore this.
    fn activate_downloaded(&mut self) {}

    /// Update the metadata (dimensions) of the download slot.
    /// Called before decoding a newly downloaded image so that `bitmap()`
    /// returns the correct width/height for the stored format.
    /// Implementations that do not support swapping may ignore this.
    fn set_downloaded_metadata(&mut self, _metadata: BitmapStorageMetadata) {}
}
