use alloc::vec::Vec;

use defmt::info;
use pov_proto::image::{DecodeMode, Encoding, decode_into_rgb8};
use pov_proto::video;

use crate::bitmap::{BitmapStorage, BitmapStorageMetadata};
use crate::storage;
use crate::storage::config::{ImageKind, ImageSlotState};

pub struct VideoPlaybackState {
    pub bytes: Vec<u8>,
    pub frame_delay_ms: u16,
    pub frame_count: u16,
    pub next_frame: u16,
}

pub enum LoadedFlashContent {
    StaticImage,
    Video(VideoPlaybackState),
}

fn metadata_from_encoding(encoding: Encoding) -> BitmapStorageMetadata {
    let (width, height) = match encoding {
        Encoding::Rgb888Deflate => (64usize, 64usize),
        Encoding::PolarRgb888Deflate { leds, radials } => {
            (leds.get() as usize, radials.get() as usize)
        }
    };
    BitmapStorageMetadata { width, height }
}

fn parse_frame_encoding(frame_bytes: &[u8]) -> Option<Encoding> {
    if frame_bytes.len() < 5 {
        return None;
    }
    if &frame_bytes[..3] != b"POV" || frame_bytes[3] != 1 {
        return None;
    }
    postcard::take_from_bytes::<Encoding>(&frame_bytes[4..])
        .ok()
        .map(|(enc, _)| enc)
}

fn decode_image_payload_into_store(
    payload: &[u8],
    encoding: Encoding,
    bitmap_store: &mut impl BitmapStorage,
    decode_scratch: &mut [u8],
) -> bool {
    bitmap_store.set_downloaded_metadata(metadata_from_encoding(encoding));
    match bitmap_store.bitmap_mut(0) {
        Ok(mut writable) => match decode_into_rgb8(
            payload,
            decode_scratch,
            writable.pixels_mut(),
            DecodeMode::ExactPixels,
        ) {
            Ok(_) => {
                bitmap_store.activate_downloaded();
                true
            }
            Err(err) => {
                info!("decode_image_payload_into_store decode error: {:?}", err);
                false
            }
        },
        Err(_) => false,
    }
}

pub fn advance_video_frame(
    playback: &mut VideoPlaybackState,
    bitmap_store: &mut impl BitmapStorage,
    decode_scratch: &mut [u8],
) -> bool {
    if playback.frame_count == 0 {
        return false;
    }
    let index = playback.next_frame % playback.frame_count;
    let frame_bytes = match video::frame_at(&playback.bytes, index) {
        Ok(b) => b,
        Err(err) => {
            info!("advance_video_frame frame lookup error: {:?}", err);
            return false;
        }
    };
    let encoding = match parse_frame_encoding(frame_bytes) {
        Some(enc) => enc,
        None => return false,
    };
    if !decode_image_payload_into_store(frame_bytes, encoding, bitmap_store, decode_scratch) {
        return false;
    }
    playback.next_frame = (index + 1) % playback.frame_count;
    true
}

/// Loads and decodes an image from flash slot `slot` into `bitmap_store`.
///
/// Sets the bitmap metadata (width/height) derived from the stored encoding
/// before decoding, so the storage reports correct dimensions for subsequent
/// reads. Returns `true` on success.
pub async fn load_flash_slot(
    slot: usize,
    bitmap_store: &mut impl BitmapStorage,
    decode_scratch: &mut [u8],
) -> Option<LoadedFlashContent> {
    let state = storage::get_slot_state(slot).await;
    let (kind, encoding) = match state {
        ImageSlotState::Valid { kind, encoding, .. } => (kind, encoding),
        _ => return None,
    };

    match storage::read_slot_data(slot).await {
        Ok(bytes) => match kind {
            ImageKind::Static => {
                if decode_image_payload_into_store(&bytes, encoding, bitmap_store, decode_scratch) {
                    Some(LoadedFlashContent::StaticImage)
                } else {
                    None
                }
            }
            ImageKind::Video => {
                let header = match video::parse_header(&bytes) {
                    Ok(h) => h,
                    Err(err) => {
                        info!(
                            "load_flash_slot: slot {} invalid video header: {:?}",
                            slot, err
                        );
                        return None;
                    }
                };
                let first = match video::frame_at(&bytes, 0) {
                    Ok(f) => f,
                    Err(err) => {
                        info!(
                            "load_flash_slot: slot {} first frame error: {:?}",
                            slot, err
                        );
                        return None;
                    }
                };
                let first_encoding = match parse_frame_encoding(first) {
                    Some(enc) => enc,
                    None => return None,
                };
                if !decode_image_payload_into_store(
                    first,
                    first_encoding,
                    bitmap_store,
                    decode_scratch,
                ) {
                    return None;
                }
                Some(LoadedFlashContent::Video(VideoPlaybackState {
                    bytes,
                    frame_delay_ms: header.frame_delay_ms.max(10),
                    frame_count: header.frame_count,
                    next_frame: if header.frame_count > 1 { 1 } else { 0 },
                }))
            }
        },
        Err(()) => {
            info!("load_flash_slot: slot {} read error", slot);
            None
        }
    }
}

/// Scans stored images in preferred order at boot and loads the first valid image.
///
/// Returns the image id that was loaded, or `None` if no valid image was found.
pub async fn boot_restore(
    bitmap_store: &mut impl BitmapStorage,
    decode_scratch: &mut [u8],
) -> Option<usize> {
    let active_image = storage::get_active_slot().await;
    let mut ids_to_try = storage::list_image_ids().await.unwrap_or_default();
    ids_to_try.reverse();

    if let Some(active) = active_image {
        ids_to_try.retain(|id| *id != active);
        ids_to_try.insert(0, active);
    }

    for &slot in &ids_to_try {
        let state = storage::get_slot_state(slot).await;
        info!("led_task:boot slot={} state={:?}", slot, state);
        if let ImageSlotState::Valid { .. } = state {
            if load_flash_slot(slot, bitmap_store, decode_scratch)
                .await
                .is_some()
            {
                info!("led_task:boot restored flash slot {}", slot);
                return Some(slot);
            } else {
                info!("led_task:boot failed to load flash slot {}", slot);
            }
        }
    }
    None
}
