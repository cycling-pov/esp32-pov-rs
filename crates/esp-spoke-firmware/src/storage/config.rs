use defmt::{info, warn};
use ekv::ReadError;
use pov_proto::image::Encoding;
use serde::{Deserialize, Serialize};

use super::ekv_flash::{EkvDatabase, KEY_ACTIVE_SLOT, meta_key};

// ── Value types ───────────────────────────────────────────────────────────────

/// Logical kind of image stored in a slot.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, defmt::Format)]
pub enum ImageKind {
    /// A single static display image.
    Static,
    /// A multi-frame video / animation.
    Video,
}

/// State of a single image slot.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, defmt::Format)]
pub enum ImageSlotState {
    /// Slot contains no image data.
    Empty,
    /// A write is in progress (or was interrupted). Treated as empty on boot.
    Writing,
    /// A complete image is stored.
    Valid {
        chunk_count: u16,
        total_bytes: u32,
        kind: ImageKind,
        encoding: Encoding,
    },
}

// ── Slot metadata record ──────────────────────────────────────────────────────

/// Per-slot metadata stored in ekv under `meta_key(slot)`.
///
/// The `base_key` field records the slot index that is used as the namespace
/// prefix when constructing individual chunk keys (`chunk_key(base_key, n)`),
/// allowing the metadata to self-describe which chunk keys belong to it.
#[derive(Serialize, Deserialize)]
pub struct SlotMetadata {
    /// Completion state of the slot.
    pub state: ImageSlotState,
    /// Number of chunks written (valid when state == Valid).
    pub chunk_count: u16,
    /// Slot index — the base key used to namespace the chunk records.
    pub base_key: u8,
}

impl SlotMetadata {
    /// Maximum postcard-serialized length for a `SlotMetadata` value.
    /// Calculated conservatively: tag(1) + state_variant(1) + chunk_count(3)
    /// + total_bytes(5) + kind(1) + encoding(8) + base_key(1) = ~20 bytes.
    ///
    /// Rounded up to 64 for safety.
    pub const MAX_SERIALIZED_LEN: usize = 64;

    #[allow(clippy::result_unit_err)]
    pub fn serialize_to<'a>(
        &self,
        buf: &'a mut [u8; Self::MAX_SERIALIZED_LEN],
    ) -> Result<&'a [u8], ()> {
        postcard::to_slice(self, buf).map(|s| &*s).map_err(|_| ())
    }

    pub fn deserialize(bytes: &[u8]) -> Option<Self> {
        postcard::from_bytes(bytes).ok()
    }
}

// ── Database helpers ──────────────────────────────────────────────────────────

/// Return which slot is currently active, or `None` if not set.
pub async fn get_active_slot(db: &EkvDatabase) -> Option<u8> {
    info!("config:get_active_slot");
    let rtx = db.read_transaction().await;
    let mut buf = [0u8; 1];
    let slot = match rtx.read(KEY_ACTIVE_SLOT, &mut buf).await {
        Ok(1) => Some(buf[0]),
        Ok(_) | Err(ReadError::KeyNotFound) => None,
        Err(e) => {
            warn!(
                "config:get_active_slot read error: {:?}",
                defmt::Debug2Format(&e)
            );
            None
        }
    };
    info!("config:get_active_slot result={:?}", slot);
    slot
}

/// Persist the active slot index.
///
/// Opens a short write transaction, writes `KEY_ACTIVE_SLOT`, and commits.
pub async fn set_active_slot(db: &EkvDatabase, slot: u8) -> Result<(), ()> {
    info!("config:set_active_slot slot={}", slot);
    let mut wtx = db.write_transaction().await;
    wtx.write(KEY_ACTIVE_SLOT, &[slot]).await.map_err(|_| {
        warn!("config:set_active_slot write error slot={}", slot);
    })?;
    wtx.commit().await.map_err(|_| {
        warn!("config:set_active_slot commit error slot={}", slot);
    })
}

/// Return the stored state of `slot`, defaulting to `Empty` if not present.
pub async fn get_slot_state(db: &EkvDatabase, slot: usize) -> ImageSlotState {
    info!("config:get_slot_state slot={}", slot);
    let rtx = db.read_transaction().await;
    let mut buf = [0u8; SlotMetadata::MAX_SERIALIZED_LEN];
    let state = match rtx.read(&meta_key(slot), &mut buf).await {
        Ok(len) => SlotMetadata::deserialize(&buf[..len])
            .map(|m| m.state)
            .unwrap_or(ImageSlotState::Empty),
        Err(ReadError::KeyNotFound) => ImageSlotState::Empty,
        Err(e) => {
            warn!(
                "config:get_slot_state read error slot={}: {:?}",
                slot,
                defmt::Debug2Format(&e)
            );
            ImageSlotState::Empty
        }
    };
    info!("config:get_slot_state slot={} state={:?}", slot, state);
    state
}

/// Persist the state of `slot`.
///
/// Reads existing metadata (to preserve `chunk_count` / `base_key`), then
/// opens a short write transaction and commits the updated record.
pub async fn set_slot_state(
    db: &EkvDatabase,
    slot: usize,
    state: ImageSlotState,
) -> Result<(), ()> {
    info!("config:set_slot_state slot={} state={:?}", slot, state);
    // Preserve any existing chunk_count; default to 0 for new records.
    let rtx = db.read_transaction().await;
    let mut read_buf = [0u8; SlotMetadata::MAX_SERIALIZED_LEN];
    let existing_chunk_count = match rtx.read(&meta_key(slot), &mut read_buf).await {
        Ok(len) => SlotMetadata::deserialize(&read_buf[..len])
            .map(|m| m.chunk_count)
            .unwrap_or(0),
        _ => 0,
    };
    drop(rtx);

    let meta = SlotMetadata {
        state,
        chunk_count: existing_chunk_count,
        base_key: slot as u8,
    };
    let mut write_buf = [0u8; SlotMetadata::MAX_SERIALIZED_LEN];
    let serialized = meta.serialize_to(&mut write_buf)?;

    let mut wtx = db.write_transaction().await;
    wtx.write(&meta_key(slot), serialized).await.map_err(|_| {
        warn!("config:set_slot_state write error slot={}", slot);
    })?;
    wtx.commit().await.map_err(|_| {
        warn!("config:set_slot_state commit error slot={}", slot);
    })
}
