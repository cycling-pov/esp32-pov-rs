use core::ops::Range;

use embassy_embedded_hal::adapter::BlockingAsync;
use esp_storage::FlashStorage;

pub mod config;
pub mod image_file;

// At least one flash-size feature must be enabled (checked at compile time).
// Note: flash-4mb and flash-16mb are mutually exclusive at runtime; enabling
// both (e.g. via --all-features) is only expected in tooling contexts such as
// cargo clippy --all-features, where the constants resolve to the same values.
#[cfg(not(any(feature = "flash-4mb", feature = "flash-16mb")))]
compile_error!(
    "esp-spoke-firmware: one of the 'flash-4mb' or 'flash-16mb' features must be enabled"
);

/// Async flash type used throughout the storage module.
pub type AsyncFlash<'d> = BlockingAsync<FlashStorage<'d>>;

/// Flash range for the `pov_config` partition (64 KB).
#[cfg(any(feature = "flash-4mb", feature = "flash-16mb"))]
pub const CONFIG_FLASH_RANGE: Range<u32> = 0x320000..0x330000;

/// Flash range for the `pov_img_0` partition (100 KB).
#[cfg(any(feature = "flash-4mb", feature = "flash-16mb"))]
pub const IMG0_FLASH_RANGE: Range<u32> = 0x330000..0x349000;

/// Flash range for the `pov_img_1` partition (100 KB).
#[cfg(any(feature = "flash-4mb", feature = "flash-16mb"))]
pub const IMG1_FLASH_RANGE: Range<u32> = 0x349000..0x362000;

/// Maximum bytes per queue push; kept well below the 4096-byte page limit.
pub const CHUNK_SIZE: usize = 3840;
