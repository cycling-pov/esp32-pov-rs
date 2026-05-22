mod in_memory_storage;
mod storage;

#[cfg(feature = "builtin-image")]
pub use in_memory_storage::{BUILTIN_IMAGES, GENERATED_BITMAP, generated_image_storage};
pub use in_memory_storage::{
    GENERATED_BITMAP_METADATA, InMemoryImageStorage, MAX_POLAR_PIXEL_COUNT, SwappingImageStorage,
    generated_swapping_storage,
};
pub use storage::{Bitmap, BitmapError, BitmapMut, BitmapStorage, BitmapStorageMetadata};
