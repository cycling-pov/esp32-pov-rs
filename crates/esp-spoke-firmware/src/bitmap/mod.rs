mod in_memory_storage;
mod storage;

pub use in_memory_storage::{
    BUILTIN_IMAGES, GENERATED_BITMAP, GENERATED_BITMAP_METADATA, InMemoryImageStorage,
    SwappingImageStorage, generated_image_storage, generated_swapping_storage,
};
pub use storage::{Bitmap, BitmapError, BitmapMut, BitmapStorage, BitmapStorageMetadata};
