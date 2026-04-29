use defmt::{info, warn};
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

/// Manages the config partition (slot metadata, active slot index).
///
/// Flash is not owned; callers pass `&mut AsyncFlash<'_>` for each operation,
/// allowing a single flash instance to be shared across multiple stores.
pub struct ConfigStore;

impl Default for ConfigStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigStore {
    pub fn new() -> Self {
        Self
    }

    /// Return which slot is currently active, or `None` if not set.
    pub async fn get_active_slot(
        &mut self,
        flash: &mut AsyncFlash<'_>,
        buf: &mut [u8],
    ) -> Option<u8> {
        info!("config:get_active_slot start");
        let mut storage =
            MapStorage::<u8, _, _>::new(flash, MapConfig::new(CONFIG_FLASH_RANGE), NoCache::new());
        let slot = storage
            .fetch_item::<ActiveSlotIndex>(buf, &KEY_ACTIVE_SLOT)
            .await
            .ok()
            .flatten()
            .map(|v| v.0);
        info!("config:get_active_slot result={:?}", slot);
        slot
    }

    /// Persist the active slot index.
    pub async fn set_active_slot(
        &mut self,
        flash: &mut AsyncFlash<'_>,
        slot: u8,
        buf: &mut [u8],
    ) -> Result<(), ()> {
        info!("config:set_active_slot slot={}", slot);
        let mut storage =
            MapStorage::<u8, _, _>::new(flash, MapConfig::new(CONFIG_FLASH_RANGE), NoCache::new());
        let result = storage
            .store_item(buf, &KEY_ACTIVE_SLOT, &ActiveSlotIndex(slot))
            .await
            .map_err(|_| ());
        if result.is_err() {
            warn!("config:set_active_slot failed slot={}", slot);
        }
        result
    }

    /// Return the stored state of `slot` (0 or 1), defaulting to `Empty`.
    pub async fn get_slot_state(
        &mut self,
        flash: &mut AsyncFlash<'_>,
        slot: usize,
        buf: &mut [u8],
    ) -> ImageSlotState {
        info!("config:get_slot_state slot={}", slot);
        let key = if slot == 0 {
            KEY_SLOT0_STATE
        } else {
            KEY_SLOT1_STATE
        };
        let mut storage =
            MapStorage::<u8, _, _>::new(flash, MapConfig::new(CONFIG_FLASH_RANGE), NoCache::new());
        let state = match storage.fetch_item::<ImageSlotState>(buf, &key).await {
            Ok(Some(state)) => state,
            _ => ImageSlotState::Empty,
        };
        info!("config:get_slot_state slot={} state={:?}", slot, state);
        state
    }

    /// Persist the state of `slot` (0 or 1).
    pub async fn set_slot_state(
        &mut self,
        flash: &mut AsyncFlash<'_>,
        slot: usize,
        state: &ImageSlotState,
        buf: &mut [u8],
    ) -> Result<(), ()> {
        info!("config:set_slot_state slot={} state={:?}", slot, state);
        let key = if slot == 0 {
            KEY_SLOT0_STATE
        } else {
            KEY_SLOT1_STATE
        };
        let mut storage =
            MapStorage::<u8, _, _>::new(flash, MapConfig::new(CONFIG_FLASH_RANGE), NoCache::new());
        let result = storage.store_item(buf, &key, state).await.map_err(|_| ());
        if result.is_err() {
            warn!(
                "config:set_slot_state failed slot={} state={:?}",
                slot, state
            );
        }
        result
    }

    /// Erase all config data (e.g. factory reset).
    pub async fn erase_all(&mut self, flash: &mut AsyncFlash<'_>) -> Result<(), ()> {
        info!("config:erase_all start");
        let mut storage =
            MapStorage::<u8, _, _>::new(flash, MapConfig::new(CONFIG_FLASH_RANGE), NoCache::new());
        let result = storage.erase_all().await.map_err(|_| ());
        if result.is_err() {
            warn!("config:erase_all failed");
        }
        result
    }
}
