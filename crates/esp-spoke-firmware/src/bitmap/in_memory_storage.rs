use alloc::boxed::Box;

use smart_leds_trait::RGB8;

use crate::bitmap::{Bitmap, BitmapError, BitmapMut, BitmapStorage, BitmapStorageMetadata};

include!(concat!(env!("OUT_DIR"), "/asset_bitmap.rs"));

pub static BUILTIN_IMAGES: [[RGB8; GENERATED_BITMAP_PIXEL_COUNT]; 1] = [GENERATED_BITMAP];

pub fn generated_image_storage(
) -> Box<InMemoryImageStorage<1, GENERATED_BITMAP_PIXEL_COUNT>> {
    Box::new(InMemoryImageStorage::new(GENERATED_BITMAP_METADATA, &BUILTIN_IMAGES))
}

pub struct InMemoryImageStorage<const IMAGE_COUNT: usize, const PIXEL_COUNT: usize> {
    metadata: BitmapStorageMetadata,
    images: &'static [[RGB8; PIXEL_COUNT]; IMAGE_COUNT],
    writable_image: [RGB8; PIXEL_COUNT],
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
            writable_image: [RGB8::default(); PIXEL_COUNT],
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
        IMAGE_COUNT
    }

    fn bitmap(&self, index: usize) -> Result<Bitmap<'_>, BitmapError> {
        if let Some(image) = self.images.get(index) {
            return Ok(Bitmap::new(self.metadata, image));
        }

        Err(BitmapError::InvalidIndex {
            index,
            bitmap_count: self.bitmap_count(),
        })
    }

    fn bitmap_mut(&mut self, index: usize) -> Result<BitmapMut<'_>, BitmapError> {
        if index == IMAGE_COUNT {
            return Ok(BitmapMut::new(self.metadata, &mut self.writable_image));
        }

        if index < IMAGE_COUNT {
            return Err(BitmapError::NotWritable { index });
        }

        Err(BitmapError::InvalidIndex {
            index,
            bitmap_count: self.bitmap_count(),
        })
    }
}
