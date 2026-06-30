use defmt::{info, warn};
use ekv::ReadError;
use pov_proto::image::Encoding;
use serde::{Deserialize, Serialize};

use super::ekv_flash::{
    EkvDatabase, KEY_ACTIVE_SLOT, KEY_SENSOR_CONFIG, KEY_STORAGE_INDEX, KEY_STORAGE_SCHEMA_VERSION,
    meta_key,
};

pub const STORAGE_SCHEMA_VERSION: u8 = 2;
pub const MAX_TRACKED_IMAGES: usize = 32;

// -- Value types ---------------------------------------------------------------

#[derive(Clone, Copy, Debug, Serialize, Deserialize, defmt::Format)]
pub enum ImageKind {
    Static,
    Video,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, defmt::Format)]
pub enum ImageSlotState {
    Empty,
    Writing,
    Valid {
        chunk_count: u16,
        total_bytes: u32,
        kind: ImageKind,
        encoding: Encoding,
    },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, defmt::Format)]
pub struct SensorConfig {
    pub hall_offset_0_degrees: f32,
    pub hall_offset_1_degrees: f32,
    pub imu_offset_degrees: f32,
}

impl SensorConfig {
    pub const MAX_SERIALIZED_LEN: usize = 32;

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

impl Default for SensorConfig {
    fn default() -> Self {
        Self {
            hall_offset_0_degrees: 0.0,
            hall_offset_1_degrees: 0.0,
            imu_offset_degrees: 0.0,
        }
    }
}

// -- Image metadata ------------------------------------------------------------

#[derive(Serialize, Deserialize)]
pub struct SlotMetadata {
    pub image_id: u32,
    pub state: ImageSlotState,
    pub chunk_count: u16,
}

impl SlotMetadata {
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

// -- Storage index -------------------------------------------------------------

#[derive(Serialize, Deserialize)]
pub struct StorageIndex {
    pub next_image_id: u32,
    pub used_bytes: u32,
    pub active_image_id: Option<u32>,
    // Oldest -> newest for deterministic FIFO eviction.
    pub image_ids: [u32; MAX_TRACKED_IMAGES],
    pub image_count: u8,
}

impl Default for StorageIndex {
    fn default() -> Self {
        Self {
            next_image_id: 1,
            used_bytes: 0,
            active_image_id: None,
            image_ids: [0; MAX_TRACKED_IMAGES],
            image_count: 0,
        }
    }
}

impl StorageIndex {
    pub const MAX_SERIALIZED_LEN: usize = 1024;

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

    pub fn image_ids_slice(&self) -> &[u32] {
        &self.image_ids[..self.image_count as usize]
    }

    pub fn position_of(&self, image_id: u32) -> Option<usize> {
        self.image_ids_slice().iter().position(|id| *id == image_id)
    }

    pub fn remove_at(&mut self, index: usize) {
        let count = self.image_count as usize;
        if index >= count {
            return;
        }
        for i in index..count.saturating_sub(1) {
            self.image_ids[i] = self.image_ids[i + 1];
        }
        if count > 0 {
            self.image_ids[count - 1] = 0;
            self.image_count = self.image_count.saturating_sub(1);
        }
    }

    pub fn push_newest(&mut self, image_id: u32) {
        if let Some(pos) = self.position_of(image_id) {
            self.remove_at(pos);
        }

        let count = self.image_count as usize;
        if count < MAX_TRACKED_IMAGES {
            self.image_ids[count] = image_id;
            self.image_count = self.image_count.saturating_add(1);
            return;
        }

        // Index is full: evict the oldest bookkeeping entry.
        self.remove_at(0);
        let new_count = self.image_count as usize;
        self.image_ids[new_count] = image_id;
        self.image_count = self.image_count.saturating_add(1);
    }
}

// -- Schema/version helpers ----------------------------------------------------

pub async fn read_schema_version(db: &EkvDatabase) -> Option<u8> {
    let rtx = db.read_transaction().await;
    let mut buf = [0u8; 1];
    match rtx.read(KEY_STORAGE_SCHEMA_VERSION, &mut buf).await {
        Ok(1) => Some(buf[0]),
        _ => None,
    }
}

pub async fn write_schema_version(db: &EkvDatabase, version: u8) -> Result<(), ()> {
    let mut wtx = db.write_transaction().await;
    wtx.write(KEY_STORAGE_SCHEMA_VERSION, &[version])
        .await
        .map_err(|_| {
            warn!("config:write_schema_version write error v={}", version);
        })?;
    wtx.commit().await.map_err(|_| {
        warn!("config:write_schema_version commit error v={}", version);
    })
}

// -- Active image helpers ------------------------------------------------------

pub async fn get_active_slot(db: &EkvDatabase) -> Option<usize> {
    info!("config:get_active_slot");
    let rtx = db.read_transaction().await;
    let mut buf = [0u8; 4];
    let image = match rtx.read(KEY_ACTIVE_SLOT, &mut buf).await {
        Ok(4) => Some(u32::from_le_bytes(buf) as usize),
        Ok(_) | Err(ReadError::KeyNotFound) => None,
        Err(e) => {
            warn!(
                "config:get_active_slot read error: {:?}",
                defmt::Debug2Format(&e)
            );
            None
        }
    };
    info!("config:get_active_slot result={:?}", image);
    image
}

pub async fn set_active_slot(db: &EkvDatabase, image_id: usize) -> Result<(), ()> {
    info!("config:set_active_slot image_id={}", image_id);
    let mut wtx = db.write_transaction().await;
    wtx.write(KEY_ACTIVE_SLOT, &(image_id as u32).to_le_bytes())
        .await
        .map_err(|_| {
            warn!("config:set_active_slot write error image_id={}", image_id);
        })?;
    wtx.commit().await.map_err(|_| {
        warn!("config:set_active_slot commit error image_id={}", image_id);
    })
}

pub async fn clear_active_slot(db: &EkvDatabase) -> Result<(), ()> {
    let mut wtx = db.write_transaction().await;
    wtx.delete(KEY_ACTIVE_SLOT).await.map_err(|_| {
        warn!("config:clear_active_slot delete error");
    })?;
    wtx.commit().await.map_err(|_| {
        warn!("config:clear_active_slot commit error");
    })
}

// -- Sensor config helpers -----------------------------------------------------

pub async fn get_sensor_config(db: &EkvDatabase) -> SensorConfig {
    info!("config:get_sensor_config");
    let rtx = db.read_transaction().await;
    let mut buf = [0u8; SensorConfig::MAX_SERIALIZED_LEN];
    let config = match rtx.read(KEY_SENSOR_CONFIG, &mut buf).await {
        Ok(len) => SensorConfig::deserialize(&buf[..len]).unwrap_or_else(|| {
            warn!("config:get_sensor_config decode error, using defaults");
            SensorConfig::default()
        }),
        Err(ReadError::KeyNotFound) => SensorConfig::default(),
        Err(e) => {
            warn!(
                "config:get_sensor_config read error: {:?}",
                defmt::Debug2Format(&e)
            );
            SensorConfig::default()
        }
    };
    info!("config:get_sensor_config result={:?}", config);
    config
}

pub async fn set_sensor_config(db: &EkvDatabase, config: SensorConfig) -> Result<(), ()> {
    info!("config:set_sensor_config config={:?}", config);
    let mut write_buf = [0u8; SensorConfig::MAX_SERIALIZED_LEN];
    let serialized = config.serialize_to(&mut write_buf)?;

    let mut wtx = db.write_transaction().await;
    wtx.write(KEY_SENSOR_CONFIG, serialized)
        .await
        .map_err(|_| {
            warn!("config:set_sensor_config write error");
        })?;
    wtx.commit().await.map_err(|_| {
        warn!("config:set_sensor_config commit error");
    })
}

// -- Image metadata helpers ----------------------------------------------------

pub async fn get_slot_state(db: &EkvDatabase, image_id: usize) -> ImageSlotState {
    info!("config:get_slot_state image_id={}", image_id);
    let rtx = db.read_transaction().await;
    let mut buf = [0u8; SlotMetadata::MAX_SERIALIZED_LEN];
    let state = match rtx.read(&meta_key(image_id as u32), &mut buf).await {
        Ok(len) => SlotMetadata::deserialize(&buf[..len])
            .map(|m| m.state)
            .unwrap_or(ImageSlotState::Empty),
        Err(ReadError::KeyNotFound) => ImageSlotState::Empty,
        Err(e) => {
            warn!(
                "config:get_slot_state read error image_id={}: {:?}",
                image_id,
                defmt::Debug2Format(&e)
            );
            ImageSlotState::Empty
        }
    };
    info!(
        "config:get_slot_state image_id={} state={:?}",
        image_id, state
    );
    state
}

pub async fn get_slot_metadata(db: &EkvDatabase, image_id: usize) -> Option<SlotMetadata> {
    let rtx = db.read_transaction().await;
    let mut buf = [0u8; SlotMetadata::MAX_SERIALIZED_LEN];
    match rtx.read(&meta_key(image_id as u32), &mut buf).await {
        Ok(len) => SlotMetadata::deserialize(&buf[..len]),
        _ => None,
    }
}

pub async fn set_slot_state(
    db: &EkvDatabase,
    image_id: usize,
    state: ImageSlotState,
) -> Result<(), ()> {
    info!(
        "config:set_slot_state image_id={} state={:?}",
        image_id, state
    );
    let rtx = db.read_transaction().await;
    let mut read_buf = [0u8; SlotMetadata::MAX_SERIALIZED_LEN];
    let existing_chunk_count = match rtx.read(&meta_key(image_id as u32), &mut read_buf).await {
        Ok(len) => SlotMetadata::deserialize(&read_buf[..len])
            .map(|m| m.chunk_count)
            .unwrap_or(0),
        _ => 0,
    };
    drop(rtx);

    let meta = SlotMetadata {
        image_id: image_id as u32,
        state,
        chunk_count: existing_chunk_count,
    };
    let mut write_buf = [0u8; SlotMetadata::MAX_SERIALIZED_LEN];
    let serialized = meta.serialize_to(&mut write_buf)?;

    let mut wtx = db.write_transaction().await;
    wtx.write(&meta_key(image_id as u32), serialized)
        .await
        .map_err(|_| {
            warn!("config:set_slot_state write error image_id={}", image_id);
        })?;
    wtx.commit().await.map_err(|_| {
        warn!("config:set_slot_state commit error image_id={}", image_id);
    })
}

// -- Storage index helpers -----------------------------------------------------

pub async fn get_storage_index(db: &EkvDatabase) -> StorageIndex {
    let rtx = db.read_transaction().await;
    let mut buf = [0u8; StorageIndex::MAX_SERIALIZED_LEN];
    match rtx.read(KEY_STORAGE_INDEX, &mut buf).await {
        Ok(len) => StorageIndex::deserialize(&buf[..len]).unwrap_or_default(),
        _ => StorageIndex::default(),
    }
}

pub async fn set_storage_index(db: &EkvDatabase, index: &StorageIndex) -> Result<(), ()> {
    let mut write_buf = [0u8; StorageIndex::MAX_SERIALIZED_LEN];
    let serialized = index.serialize_to(&mut write_buf)?;

    let mut wtx = db.write_transaction().await;
    wtx.write(KEY_STORAGE_INDEX, serialized)
        .await
        .map_err(|_| {
            warn!("config:set_storage_index write error");
        })?;
    wtx.commit().await.map_err(|_| {
        warn!("config:set_storage_index commit error");
    })
}
