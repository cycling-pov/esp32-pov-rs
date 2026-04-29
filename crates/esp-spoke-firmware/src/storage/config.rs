use pov_proto::image::Encoding;
use sequential_storage::{
    cache::NoCache,
    map::{MapConfig, MapStorage, PostcardValue},
};
use serde::{Deserialize, Serialize};

use super::{AsyncFlash, CONFIG_FLASH_RANGE};

// ── Map keys (stored as u8 on flash) ─────────────────────────────────────────

pub const KEY_ACTIVE_SLOT: u8 = 1;
pub const KEY_SLOT0_STATE: u8 = 2;
pub const KEY_SLOT1_STATE: u8 = 3;

// ── Value types ───────────────────────────────────────────────────────────────

/// Newtype wrapper for the active image slot index (0 or 1).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, defmt::Format)]
pub struct ActiveSlotIndex(pub u8);

impl<'a> PostcardValue<'a> for ActiveSlotIndex {}

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
    /// A write is in progress (or was interrupted). Treat as empty on boot.
    Writing,
    /// A complete image is stored.
    Valid {
        chunk_count: u16,
        total_bytes: u32,
        kind: ImageKind,
        encoding: Encoding,
    },
}

impl<'a> PostcardValue<'a> for ImageSlotState {}

// ── ConfigStore ───────────────────────────────────────────────────────────────

pub struct ConfigStore<'d> {
    storage: MapStorage<u8, AsyncFlash<'d>, NoCache>,
}

impl<'d> ConfigStore<'d> {
    pub fn new(flash: AsyncFlash<'d>) -> Self {
        Self {
            storage: MapStorage::new(flash, MapConfig::new(CONFIG_FLASH_RANGE), NoCache::new()),
        }
    }

    /// Return which slot is currently active, or `None` if not set.
    pub async fn get_active_slot(&mut self, buf: &mut [u8]) -> Option<u8> {
        self.storage
            .fetch_item::<ActiveSlotIndex>(buf, &KEY_ACTIVE_SLOT)
            .await
            .ok()
            .flatten()
            .map(|v| v.0)
    }

    /// Persist the active slot index.
    pub async fn set_active_slot(&mut self, slot: u8, buf: &mut [u8]) -> Result<(), ()> {
        self.storage
            .store_item(buf, &KEY_ACTIVE_SLOT, &ActiveSlotIndex(slot))
            .await
            .map_err(|_| ())
    }

    /// Return the stored state of `slot` (0 or 1), defaulting to `Empty`.
    pub async fn get_slot_state(&mut self, slot: usize, buf: &mut [u8]) -> ImageSlotState {
        let key = if slot == 0 {
            KEY_SLOT0_STATE
        } else {
            KEY_SLOT1_STATE
        };
        match self.storage.fetch_item::<ImageSlotState>(buf, &key).await {
            Ok(Some(state)) => state,
            _ => ImageSlotState::Empty,
        }
    }

    /// Persist the state of `slot` (0 or 1).
    pub async fn set_slot_state(
        &mut self,
        slot: usize,
        state: &ImageSlotState,
        buf: &mut [u8],
    ) -> Result<(), ()> {
        let key = if slot == 0 {
            KEY_SLOT0_STATE
        } else {
            KEY_SLOT1_STATE
        };
        self.storage
            .store_item(buf, &key, state)
            .await
            .map_err(|_| ())
    }

    /// Erase all config data (e.g. factory reset).
    pub async fn erase_all(&mut self) -> Result<(), ()> {
        self.storage.erase_all().await.map_err(|_| ())
    }
}
