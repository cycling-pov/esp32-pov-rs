use defmt::info;
use pov_proto::image::{DecodeMode, Encoding, decode_into_rgb8};
use pov_proto::transfer::{CommandFrame, SpokeCommand};

use crate::bitmap::{Bitmap, BitmapStorage, BitmapStorageMetadata};
use crate::led::LedStrip;
use crate::storage;
use crate::storage::config::ImageSlotState;

/// Loads and decodes an image from flash slot `slot` into `bitmap_store`.
///
/// Sets the bitmap metadata (width/height) derived from the stored encoding
/// before decoding, so the storage reports correct dimensions for subsequent
/// reads. Returns `true` on success.
pub async fn load_flash_slot(
    slot: usize,
    bitmap_store: &mut impl BitmapStorage,
    decode_scratch: &mut [u8],
) -> bool {
    let state = storage::get_slot_state(slot).await;
    let encoding = match state {
        ImageSlotState::Valid { encoding, .. } => encoding,
        _ => return false,
    };

    let (width, height) = match encoding {
        Encoding::Rgb888Deflate => (64usize, 64usize),
        Encoding::PolarRgb888Deflate { leds, radials } => {
            (leds.get() as usize, radials.get() as usize)
        }
    };
    bitmap_store.set_downloaded_metadata(BitmapStorageMetadata { width, height });

    match storage::read_slot_data(slot).await {
        Ok(img_bytes) => {
            if let Ok(mut writable) = bitmap_store.bitmap_mut(0) {
                match decode_into_rgb8(
                    &img_bytes,
                    decode_scratch,
                    writable.pixels_mut(),
                    DecodeMode::ExactPixels,
                ) {
                    Ok(_) => {
                        bitmap_store.activate_downloaded();
                        return true;
                    }
                    Err(err) => {
                        info!("load_flash_slot: slot {} decode error: {:?}", slot, err);
                    }
                }
            }
        }
        Err(()) => info!("load_flash_slot: slot {} read error", slot),
    }
    false
}

/// Scans flash slots in preferred order at boot and loads the first valid image.
///
/// Returns the slot index that was loaded, or `None` if no valid slot was found.
pub async fn boot_restore(
    bitmap_store: &mut impl BitmapStorage,
    decode_scratch: &mut [u8],
) -> Option<usize> {
    let active_flash_slot = storage::get_active_slot().await;
    let slots_to_try: [usize; 2] = match active_flash_slot {
        Some(s) => [s as usize, (s as usize + 1) % 2],
        None => [0, 1],
    };

    for &slot in &slots_to_try {
        let state = storage::get_slot_state(slot).await;
        info!("led_task:boot slot={} state={:?}", slot, state);
        if let ImageSlotState::Valid { .. } = state {
            if load_flash_slot(slot, bitmap_store, decode_scratch).await {
                info!("led_task:boot restored flash slot {}", slot);
                return Some(slot);
            } else {
                info!("led_task:boot failed to load flash slot {}", slot);
            }
        }
    }
    None
}

/// Renders the current active bitmap to an LED strip.
///
/// Implementors update the strip framebuffer from the provided `bitmap` and
/// then call `show()`. For POV strips the render samples the polar bitmap at
/// the current angular position; for matrix strips it scales into the LED grid.
#[allow(
    async_fn_in_trait,
    reason = "RenderBitmap is an internal firmware trait"
)]
pub trait RenderBitmap: LedStrip {
    async fn render_from_bitmap(&mut self, bitmap: &Bitmap<'_>);
}

/// Applies a [`CommandFrame`] to the LED strip and bitmap state.
///
/// Handles `DisplayOff`, `NextImage`, and `RandomizeDisplay`. Calls
/// `render_from_bitmap` on the strip whenever a new image should be displayed.
pub async fn apply_led_command<L>(
    led: &mut L,
    bitmap_store: &mut impl BitmapStorage,
    current_display_slot: &mut Option<usize>,
    decode_scratch: &mut [u8],
    randomizing: &mut bool,
    frame: CommandFrame,
) where
    L: LedStrip + RenderBitmap,
{
    info!(
        "led_task:command transfer_id={} command={:?}",
        frame.transfer_id, frame.command
    );

    match frame.command {
        SpokeCommand::DisplayOff => {
            *randomizing = false;
            led.clear();
            led.show().await.expect("failed to clear LED strip");
            info!("applied DisplayOff from transfer {}", frame.transfer_id);
        }
        SpokeCommand::NextImage => {
            *randomizing = false;
            let next_slot = match *current_display_slot {
                None => Some(0usize),
                Some(0) => Some(1),
                Some(_) => None,
            };
            *current_display_slot = next_slot;
            match next_slot {
                None => {
                    bitmap_store.activate_builtin();
                    if let Ok(bitmap) = bitmap_store.bitmap(0) {
                        led.render_from_bitmap(&bitmap).await;
                    }
                }
                Some(slot) => {
                    if load_flash_slot(slot, bitmap_store, decode_scratch).await {
                        if let Ok(bitmap) = bitmap_store.bitmap(0) {
                            led.render_from_bitmap(&bitmap).await;
                        }
                    } else {
                        led.clear();
                        led.show().await.expect("failed to clear LED strip");
                    }
                }
            }
            info!(
                "applied NextImage from transfer {}: display_slot={:?}",
                frame.transfer_id, *current_display_slot
            );
        }
        SpokeCommand::RandomizeDisplay => {
            *randomizing = true;
            info!(
                "applied RandomizeDisplay from transfer {}",
                frame.transfer_id
            );
        }
    }
}
