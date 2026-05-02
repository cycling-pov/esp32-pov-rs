use alloc::boxed::Box;

use smart_leds_trait::RGB8;

use crate::bitmap::{Bitmap, BitmapError, BitmapMut, BitmapStorage, BitmapStorageMetadata};

include!(concat!(env!("OUT_DIR"), "/asset_bitmap.rs"));

/// Maximum pixel count for the download buffer: enough to hold a full polar
/// image (30 LEDs × 360 radials = 10 800 pixels) or a 64×64 Cartesian image
/// (4 096 pixels).
pub const MAX_POLAR_PIXEL_COUNT: usize = 30 * 360;

// Sanity: the built-in image must fit in the download buffer.
const _: () = assert!(
    GENERATED_BITMAP_PIXEL_COUNT <= MAX_POLAR_PIXEL_COUNT,
    "GENERATED_BITMAP_PIXEL_COUNT exceeds MAX_POLAR_PIXEL_COUNT",
);

#[cfg(feature = "builtin-image")]
pub static BUILTIN_IMAGES: [[RGB8; GENERATED_BITMAP_PIXEL_COUNT]; 1] = [GENERATED_BITMAP];

const DOWNLOADABLE_IMAGE_SLOTS: usize = 2;

#[cfg(feature = "builtin-image")]
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
/// Only present when the `builtin-image` feature is enabled.
#[cfg(feature = "builtin-image")]
#[derive(Clone, Copy)]
enum ActiveImage {
    Builtin,
    Downloaded,
}

/// A `BitmapStorage` with exactly one logical bitmap (index 0) whose pixel
/// source can be switched between a static built-in image and a single
/// in-memory download buffer.
///
/// `MAX_DOWNLOAD_PIXELS` is the capacity of the download buffer; it must be
/// large enough for the largest image format (use `MAX_POLAR_PIXEL_COUNT`).
/// Both 64×64 Cartesian images and 30×360 polar images fit within this limit.
///
/// The download buffer's active pixel count is tracked separately via
/// `downloaded_metadata`, which is updated by `set_downloaded_metadata` before
/// each new image is decoded into the buffer.
pub struct SwappingImageStorage<const MAX_DOWNLOAD_PIXELS: usize> {
    #[cfg(feature = "builtin-image")]
    builtin_metadata: BitmapStorageMetadata,
    downloaded_metadata: BitmapStorageMetadata,
    #[cfg(feature = "builtin-image")]
    builtin: &'static [RGB8],
    image_from_flash: [RGB8; MAX_DOWNLOAD_PIXELS],
    #[cfg(feature = "builtin-image")]
    active: ActiveImage,
}

#[cfg(feature = "builtin-image")]
impl<const MAX_DOWNLOAD_PIXELS: usize> SwappingImageStorage<MAX_DOWNLOAD_PIXELS> {
    pub fn new(
        builtin_metadata: BitmapStorageMetadata,
        builtin: &'static [RGB8],
    ) -> Self {
        assert!(builtin.len() == builtin_metadata.pixel_count());
        assert!(MAX_DOWNLOAD_PIXELS >= builtin_metadata.pixel_count());
        Self {
            builtin_metadata,
            downloaded_metadata: builtin_metadata,
            builtin,
            image_from_flash: [RGB8::default(); MAX_DOWNLOAD_PIXELS],
            active: ActiveImage::Builtin,
        }
    }
}

#[cfg(not(feature = "builtin-image"))]
impl<const MAX_DOWNLOAD_PIXELS: usize> SwappingImageStorage<MAX_DOWNLOAD_PIXELS> {
    pub fn new(builtin_metadata: BitmapStorageMetadata) -> Self {
        Self {
            downloaded_metadata: builtin_metadata,
            image_from_flash: [RGB8::default(); MAX_DOWNLOAD_PIXELS],
        }
    }
}

#[cfg(feature = "builtin-image")]
impl<const MAX_DOWNLOAD_PIXELS: usize> BitmapStorage for SwappingImageStorage<MAX_DOWNLOAD_PIXELS> {
    fn metadata(&self) -> BitmapStorageMetadata {
        match self.active {
            ActiveImage::Builtin => self.builtin_metadata,
            ActiveImage::Downloaded => self.downloaded_metadata,
        }
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
            ActiveImage::Builtin => Ok(Bitmap::new(self.builtin_metadata, self.builtin)),
            ActiveImage::Downloaded => {
                let pixel_count = self.downloaded_metadata.pixel_count();
                Ok(Bitmap::new(
                    self.downloaded_metadata,
                    &self.image_from_flash[..pixel_count],
                ))
            }
        }
    }

    fn bitmap_mut(&mut self, index: usize) -> Result<BitmapMut<'_>, BitmapError> {
        if index != 0 {
            return Err(BitmapError::InvalidIndex {
                index,
                bitmap_count: 1,
            });
        }
        // new_with_capacity asserts image_from_flash.len() >= downloaded_metadata.pixel_count()
        Ok(BitmapMut::new_with_capacity(
            self.downloaded_metadata,
            &mut self.image_from_flash,
        ))
    }

    fn activate_builtin(&mut self) {
        self.active = ActiveImage::Builtin;
    }

    fn activate_downloaded(&mut self) {
        self.active = ActiveImage::Downloaded;
    }

    fn set_downloaded_metadata(&mut self, metadata: BitmapStorageMetadata) {
        assert!(
            MAX_DOWNLOAD_PIXELS >= metadata.pixel_count(),
            "downloaded image pixel_count {} exceeds buffer capacity {}",
            metadata.pixel_count(),
            MAX_DOWNLOAD_PIXELS
        );
        self.downloaded_metadata = metadata;
    }
}

#[cfg(not(feature = "builtin-image"))]
impl<const MAX_DOWNLOAD_PIXELS: usize> BitmapStorage for SwappingImageStorage<MAX_DOWNLOAD_PIXELS> {
    fn metadata(&self) -> BitmapStorageMetadata {
        self.downloaded_metadata
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
        let pixel_count = self.downloaded_metadata.pixel_count();
        Ok(Bitmap::new(
            self.downloaded_metadata,
            &self.image_from_flash[..pixel_count],
        ))
    }

    fn bitmap_mut(&mut self, index: usize) -> Result<BitmapMut<'_>, BitmapError> {
        if index != 0 {
            return Err(BitmapError::InvalidIndex {
                index,
                bitmap_count: 1,
            });
        }
        Ok(BitmapMut::new_with_capacity(
            self.downloaded_metadata,
            &mut self.image_from_flash,
        ))
    }

    fn set_downloaded_metadata(&mut self, metadata: BitmapStorageMetadata) {
        assert!(
            MAX_DOWNLOAD_PIXELS >= metadata.pixel_count(),
            "downloaded image pixel_count {} exceeds buffer capacity {}",
            metadata.pixel_count(),
            MAX_DOWNLOAD_PIXELS
        );
        self.downloaded_metadata = metadata;
    }
}

/// Create a heap-allocated `SwappingImageStorage` initialised with the
/// compile-time generated built-in image.  The download buffer is sized to
/// `MAX_POLAR_PIXEL_COUNT` so it can hold both Cartesian (64×64 = 4 096) and
/// polar (30×360 = 10 800) images.
#[cfg(feature = "builtin-image")]
pub fn generated_swapping_storage() -> Box<SwappingImageStorage<MAX_POLAR_PIXEL_COUNT>> {
    Box::new(SwappingImageStorage::new(
        GENERATED_BITMAP_METADATA,
        &BUILTIN_IMAGES[0],
    ))
}

/// Create a heap-allocated `SwappingImageStorage` with an empty download buffer.
/// No built-in pixel data is allocated in ROM or RAM when `builtin-image` is off.
#[cfg(not(feature = "builtin-image"))]
pub fn generated_swapping_storage() -> Box<SwappingImageStorage<MAX_POLAR_PIXEL_COUNT>> {
    Box::new(SwappingImageStorage::new(GENERATED_BITMAP_METADATA))
}
