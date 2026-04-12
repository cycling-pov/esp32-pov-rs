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
