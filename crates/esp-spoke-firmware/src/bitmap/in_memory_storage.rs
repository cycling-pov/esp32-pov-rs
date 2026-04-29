use alloc::boxed::Box;

use smart_leds_trait::RGB8;

use crate::bitmap::{Bitmap, BitmapError, BitmapMut, BitmapStorage, BitmapStorageMetadata};

include!(concat!(env!("OUT_DIR"), "/asset_bitmap.rs"));

pub static BUILTIN_IMAGES: [[RGB8; GENERATED_BITMAP_PIXEL_COUNT]; 1] = [GENERATED_BITMAP];
const DOWNLOADABLE_IMAGE_SLOTS: usize = 2;

pub fn generated_image_storage() -> Box<InMemoryImageStorage<1, GENERATED_BITMAP_PIXEL_COUNT>> {
    Box::new(InMemoryImageStorage::new(
        GENERATED_BITMAP_METADATA,
        &BUILTIN_IMAGES,
    ))
}

pub struct InMemoryImageStorage<const IMAGE_COUNT: usize, const PIXEL_COUNT: usize> {
    metadata: BitmapStorageMetadata,
    images: &'static [[RGB8; PIXEL_COUNT]; IMAGE_COUNT],
    writable_images: [[RGB8; PIXEL_COUNT]; DOWNLOADABLE_IMAGE_SLOTS],
}

impl<const IMAGE_COUNT: usize, const PIXEL_COUNT: usize>
    InMemoryImageStorage<IMAGE_COUNT, PIXEL_COUNT>
{
    pub fn new(
        metadata: BitmapStorageMetadata,
        images: &'static [[RGB8; PIXEL_COUNT]; IMAGE_COUNT],
    ) -> Self {
        assert!(metadata.pixel_count() == PIXEL_COUNT);

        Self {
            metadata,
            images,
            writable_images: [[RGB8::default(); PIXEL_COUNT]; DOWNLOADABLE_IMAGE_SLOTS],
        }
    }
}

impl<const IMAGE_COUNT: usize, const PIXEL_COUNT: usize> BitmapStorage
    for InMemoryImageStorage<IMAGE_COUNT, PIXEL_COUNT>
{
    fn metadata(&self) -> BitmapStorageMetadata {
        self.metadata
    }

    fn bitmap_count(&self) -> usize {
        IMAGE_COUNT + DOWNLOADABLE_IMAGE_SLOTS
    }

    fn bitmap(&self, index: usize) -> Result<Bitmap<'_>, BitmapError> {
        if let Some(image) = self.images.get(index) {
            return Ok(Bitmap::new(self.metadata, image));
        }

        if let Some(image) = self.writable_images.get(index.saturating_sub(IMAGE_COUNT)) {
            return Ok(Bitmap::new(self.metadata, image));
        }

        Err(BitmapError::InvalidIndex {
            index,
            bitmap_count: self.bitmap_count(),
        })
    }

    fn bitmap_mut(&mut self, index: usize) -> Result<BitmapMut<'_>, BitmapError> {
        let bitmap_count = IMAGE_COUNT + DOWNLOADABLE_IMAGE_SLOTS;

        if index < IMAGE_COUNT {
            return Err(BitmapError::NotWritable { index });
        }

        let writable_index = index.saturating_sub(IMAGE_COUNT);
        if let Some(image) = self.writable_images.get_mut(writable_index) {
            return Ok(BitmapMut::new(self.metadata, image));
        }

        Err(BitmapError::InvalidIndex {
            index,
            bitmap_count,
        })
    }
}

// ---------------------------------------------------------------------------
// SwappingImageStorage
// ---------------------------------------------------------------------------

/// Which pixels are currently returned by `bitmap(0)`.
#[derive(Clone, Copy)]
enum ActiveImage {
    Builtin,
    Downloaded,
}

/// A `BitmapStorage` with exactly one logical bitmap (index 0) whose pixel
/// source can be switched between a static built-in image and a single
/// in-memory download buffer.
///
/// - `bitmap(0)` returns the active pixels (builtin or download buffer).
/// - `bitmap_mut(0)` always returns the download buffer for writing.
/// - `activate_builtin()` / `activate_downloaded()` select the source.
pub struct SwappingImageStorage<const PIXEL_COUNT: usize> {
    metadata: BitmapStorageMetadata,
    builtin: &'static [RGB8; PIXEL_COUNT],
    download_buf: [RGB8; PIXEL_COUNT],
    active: ActiveImage,
}

impl<const PIXEL_COUNT: usize> SwappingImageStorage<PIXEL_COUNT> {
    pub fn new(metadata: BitmapStorageMetadata, builtin: &'static [RGB8; PIXEL_COUNT]) -> Self {
        assert!(metadata.pixel_count() == PIXEL_COUNT);
        Self {
            metadata,
            builtin,
            download_buf: [RGB8::default(); PIXEL_COUNT],
            active: ActiveImage::Builtin,
        }
    }
}

impl<const PIXEL_COUNT: usize> BitmapStorage for SwappingImageStorage<PIXEL_COUNT> {
    fn metadata(&self) -> BitmapStorageMetadata {
        self.metadata
    }

    fn bitmap_count(&self) -> usize {
        1
    }

    fn bitmap(&self, index: usize) -> Result<Bitmap<'_>, BitmapError> {
        if index != 0 {
            return Err(BitmapError::InvalidIndex {
                index,
                bitmap_count: 1,
            });
        }
        match self.active {
            ActiveImage::Builtin => Ok(Bitmap::new(self.metadata, self.builtin)),
            ActiveImage::Downloaded => Ok(Bitmap::new(self.metadata, &self.download_buf)),
        }
    }

    fn bitmap_mut(&mut self, index: usize) -> Result<BitmapMut<'_>, BitmapError> {
        if index != 0 {
            return Err(BitmapError::InvalidIndex {
                index,
                bitmap_count: 1,
            });
        }
        Ok(BitmapMut::new(self.metadata, &mut self.download_buf))
    }

    fn activate_builtin(&mut self) {
        self.active = ActiveImage::Builtin;
    }

    fn activate_downloaded(&mut self) {
        self.active = ActiveImage::Downloaded;
    }
}

/// Create a heap-allocated `SwappingImageStorage` initialised with the
/// compile-time generated built-in image.
pub fn generated_swapping_storage() -> Box<SwappingImageStorage<GENERATED_BITMAP_PIXEL_COUNT>> {
    Box::new(SwappingImageStorage::new(
        GENERATED_BITMAP_METADATA,
        &BUILTIN_IMAGES[0],
    ))
}
