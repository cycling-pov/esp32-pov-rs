#![no_std]

/// Allows `alloc::vec::Vec` in the `image-encode` path without requiring the
/// consumer binary to explicitly enable an allocator crate.
#[cfg(feature = "image-encode")]
extern crate alloc;

pub mod image;
pub mod transfer;
