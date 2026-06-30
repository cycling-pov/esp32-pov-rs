use alloc::vec::Vec;
use core::ops::Range;

use crc32fast::Hasher as Crc32Hasher;
use defmt::{error, info, warn};
use ekv::{Config, Database, MountError};
use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use esp_bootloader_esp_idf::partitions;
use esp_storage::FlashStorage;
use pov_proto::image::Encoding;
use pov_proto::transfer::DownloadKind;
use pov_proto::video;

use self::config::{ImageKind, ImageSlotState, SensorConfig, SlotMetadata, StorageIndex};
use self::ekv_flash::EkvFlash;

pub mod config;
pub mod ekv_flash;
pub mod image_file;

pub const CHUNK_SIZE: usize = 3840;

const STORAGE_RESERVED_BYTES: u32 = 16 * 1024;
const ESTIMATED_PER_CHUNK_OVERHEAD: u32 = 32;
const ESTIMATED_PER_IMAGE_OVERHEAD: u32 = 128;

#[derive(Clone, Copy)]
pub struct StorageStats {
    pub total_bytes: u32,
    pub used_bytes: u32,
    pub free_bytes: u32,
    pub image_count: usize,
    pub active_image_id: Option<usize>,
}

struct WriteState {
    crc: Option<Crc32Hasher>,
    chunk_count: u16,
    header: Option<[u8; 16]>,
}

struct CommitContext<'a> {
    total_capacity_bytes: u32,
    write: &'a mut WriteState,
}

enum StorageRequest {
    GetActiveSlot,
    SetActiveSlot(usize),
    GetSensorConfig,
    SetSensorConfig(SensorConfig),
    GetSlotState(usize),
    SetSlotState(usize, ImageSlotState),
    ReadSlotData(usize),
    ListImageIds,
    GetStorageStats,
    BeginSlotWrite {
        expected_bytes: u32,
    },
    WriteSlotChunk {
        slot: usize,
        chunk_num: u16,
        data: Vec<u8>,
    },
    CommitSlot {
        slot: usize,
        expected_crc32: u32,
        total_bytes: u32,
        kind: DownloadKind,
        chunk_count: u16,
    },
    AbortSlot {
        slot: usize,
        chunk_count: u16,
    },
}

enum StorageResponse {
    ActiveSlot(Option<usize>),
    SetActiveSlot(Result<(), ()>),
    SensorConfig(SensorConfig),
    SetSensorConfig(Result<(), ()>),
    SlotState(ImageSlotState),
    SetSlotState(Result<(), ()>),
    ReadSlotData(Result<Vec<u8>, ()>),
    ListImageIds(Vec<usize>),
    StorageStats(StorageStats),
    BeginSlotWrite(Result<usize, ()>),
    WriteSlotChunk(Result<(), ()>),
    CommitSlot(Result<(), ()>),
    AbortSlot(Result<(), ()>),
}

static STORAGE_REQUEST_CHANNEL: Channel<CriticalSectionRawMutex, StorageRequest, 4> =
    Channel::new();
static STORAGE_RESPONSE_CHANNEL: Channel<CriticalSectionRawMutex, StorageResponse, 4> =
    Channel::new();

async fn rpc(req: StorageRequest) -> StorageResponse {
    STORAGE_REQUEST_CHANNEL.send(req).await;
    STORAGE_RESPONSE_CHANNEL.receive().await
}

pub async fn get_active_slot() -> Option<usize> {
    match rpc(StorageRequest::GetActiveSlot).await {
        StorageResponse::ActiveSlot(slot) => slot,
        _ => {
            warn!("storage:rpc get_active_slot unexpected response");
            None
        }
    }
}

pub async fn set_active_slot(slot: usize) -> Result<(), ()> {
    match rpc(StorageRequest::SetActiveSlot(slot)).await {
        StorageResponse::SetActiveSlot(result) => result,
        _ => {
            warn!("storage:rpc set_active_slot unexpected response");
            Err(())
        }
    }
}

pub async fn get_sensor_config() -> SensorConfig {
    match rpc(StorageRequest::GetSensorConfig).await {
        StorageResponse::SensorConfig(config) => config,
        _ => {
            warn!("storage:rpc get_sensor_config unexpected response");
            SensorConfig::default()
        }
    }
}

pub async fn set_sensor_config(config: SensorConfig) -> Result<(), ()> {
    match rpc(StorageRequest::SetSensorConfig(config)).await {
        StorageResponse::SetSensorConfig(result) => result,
        _ => {
            warn!("storage:rpc set_sensor_config unexpected response");
            Err(())
        }
    }
}

pub async fn get_slot_state(slot: usize) -> ImageSlotState {
    match rpc(StorageRequest::GetSlotState(slot)).await {
        StorageResponse::SlotState(state) => state,
        _ => {
            warn!("storage:rpc get_slot_state unexpected response");
            ImageSlotState::Empty
        }
    }
}

pub async fn set_slot_state(slot: usize, state: ImageSlotState) -> Result<(), ()> {
    match rpc(StorageRequest::SetSlotState(slot, state)).await {
        StorageResponse::SetSlotState(result) => result,
        _ => {
            warn!("storage:rpc set_slot_state unexpected response");
            Err(())
        }
    }
}

pub async fn read_slot_data(slot: usize) -> Result<Vec<u8>, ()> {
    match rpc(StorageRequest::ReadSlotData(slot)).await {
        StorageResponse::ReadSlotData(result) => result,
        _ => {
            warn!("storage:rpc read_slot_data unexpected response");
            Err(())
        }
    }
}

pub async fn list_image_ids() -> Result<Vec<usize>, ()> {
    match rpc(StorageRequest::ListImageIds).await {
        StorageResponse::ListImageIds(ids) => Ok(ids),
        _ => {
            warn!("storage:rpc list_image_ids unexpected response");
            Err(())
        }
    }
}

pub async fn get_storage_stats() -> Result<StorageStats, ()> {
    match rpc(StorageRequest::GetStorageStats).await {
        StorageResponse::StorageStats(stats) => Ok(stats),
        _ => {
            warn!("storage:rpc get_storage_stats unexpected response");
            Err(())
        }
    }
}

pub async fn begin_slot_write(expected_bytes: u32) -> Result<usize, ()> {
    match rpc(StorageRequest::BeginSlotWrite { expected_bytes }).await {
        StorageResponse::BeginSlotWrite(result) => result,
        _ => {
            warn!("storage:rpc begin_slot_write unexpected response");
            Err(())
        }
    }
}

pub async fn write_slot_chunk(slot: usize, chunk_num: u16, data: &[u8]) -> Result<(), ()> {
    match rpc(StorageRequest::WriteSlotChunk {
        slot,
        chunk_num,
        data: data.to_vec(),
    })
    .await
    {
        StorageResponse::WriteSlotChunk(result) => result,
        _ => {
            warn!("storage:rpc write_slot_chunk unexpected response");
            Err(())
        }
    }
}

pub async fn commit_slot(
    slot: usize,
    expected_crc32: u32,
    total_bytes: u32,
    kind: DownloadKind,
    chunk_count: u16,
) -> Result<(), ()> {
    match rpc(StorageRequest::CommitSlot {
        slot,
        expected_crc32,
        total_bytes,
        kind,
        chunk_count,
    })
    .await
    {
        StorageResponse::CommitSlot(result) => result,
        _ => {
            warn!("storage:rpc commit_slot unexpected response");
            Err(())
        }
    }
}

pub async fn abort_slot(slot: usize, chunk_count: u16) -> Result<(), ()> {
    match rpc(StorageRequest::AbortSlot { slot, chunk_count }).await {
        StorageResponse::AbortSlot(result) => result,
        _ => {
            warn!("storage:rpc abort_slot unexpected response");
            Err(())
        }
    }
}

fn find_partition_range(table: &partitions::PartitionTable<'_>, label: &str) -> Option<Range<u32>> {
    table
        .iter()
        .find(|e| e.label_as_str() == label)
        .map(|e| e.offset()..e.offset() + e.len())
}

fn estimate_image_footprint(total_bytes: u32, chunk_count: u16) -> u32 {
    total_bytes
        .saturating_add(ESTIMATED_PER_IMAGE_OVERHEAD)
        .saturating_add((chunk_count as u32).saturating_mul(ESTIMATED_PER_CHUNK_OVERHEAD))
}

fn free_bytes(total_capacity: u32, used_bytes: u32) -> u32 {
    total_capacity.saturating_sub(used_bytes)
}

async fn evict_until_capacity(
    db: &ekv_flash::EkvDatabase,
    index: &mut StorageIndex,
    total_capacity: u32,
    required_bytes: u32,
) -> Result<(), ()> {
    while free_bytes(total_capacity, index.used_bytes) < required_bytes {
        let victim_pos = index
            .image_ids_slice()
            .iter()
            .position(|id| Some(*id) != index.active_image_id);

        let Some(victim_pos) = victim_pos else {
            warn!(
                "storage:evict no removable image required={} free={}",
                required_bytes,
                free_bytes(total_capacity, index.used_bytes)
            );
            return Err(());
        };

        let victim = index.image_ids_slice()[victim_pos];
        let Some(meta) = config::get_slot_metadata(db, victim as usize).await else {
            index.remove_at(victim_pos);
            continue;
        };

        let (total_bytes, chunk_count) = match meta.state {
            ImageSlotState::Valid {
                total_bytes,
                chunk_count,
                ..
            } => (total_bytes, chunk_count),
            _ => (0, meta.chunk_count),
        };

        info!(
            "storage:evict image_id={} bytes={} chunks={}",
            victim, total_bytes, chunk_count
        );
        image_file::purge_image(db, victim, chunk_count).await?;

        index.remove_at(victim_pos);
        index.used_bytes = index
            .used_bytes
            .saturating_sub(estimate_image_footprint(total_bytes, chunk_count));

        if index.active_image_id == Some(victim) {
            index.active_image_id = None;
            config::clear_active_slot(db).await.ok();
        }
    }

    Ok(())
}

#[embassy_executor::task]
pub async fn storage_task(flash: esp_hal::peripherals::FLASH<'static>) -> ! {
    info!("storage:task started");

    let mut flash_storage = unsafe { FlashStorage::new(flash).multicore_ignore() };
    info!("storage:task flash multicore strategy=ignore (core1 in IRAM spin)");

    let mut partition_table_raw = [0u8; partitions::PARTITION_TABLE_MAX_LEN];
    let partition_table =
        partitions::read_partition_table(&mut flash_storage, &mut partition_table_raw)
            .expect("storage:task failed to read partition table");

    let pov_store_range =
        find_partition_range(&partition_table, "pov_store").unwrap_or_else(|| {
            error!("storage:task partition 'pov_store' not found");
            panic!()
        });

    let raw_capacity = pov_store_range.end - pov_store_range.start;
    let total_capacity_bytes = raw_capacity.saturating_sub(STORAGE_RESERVED_BYTES);

    info!(
        "storage:task pov_store={:#x}..{:#x} usable_bytes={}",
        pov_store_range.start, pov_store_range.end, total_capacity_bytes
    );

    let ekv_flash = EkvFlash::new(flash_storage, pov_store_range.start, raw_capacity);
    let db: ekv_flash::EkvDatabase = Database::new(ekv_flash, Config::default());

    match db.mount().await {
        Ok(()) => info!("storage:task ekv mounted"),
        Err(MountError::Corrupted) => {
            warn!("storage:task ekv corrupted, formatting");
            db.format().await.expect("storage:task ekv format failed");
            db.mount().await.expect("storage:task ekv re-mount failed");
        }
        Err(e) => {
            error!(
                "storage:task ekv mount error: {:?}",
                defmt::Debug2Format(&e)
            );
            panic!()
        }
    }

    if config::read_schema_version(&db).await != Some(config::STORAGE_SCHEMA_VERSION) {
        warn!("storage:task schema mismatch, resetting storage");
        db.format()
            .await
            .expect("storage:task schema format failed");
        db.mount()
            .await
            .expect("storage:task schema re-mount failed");
        config::write_schema_version(&db, config::STORAGE_SCHEMA_VERSION)
            .await
            .ok();
        config::set_storage_index(&db, &StorageIndex::default())
            .await
            .ok();
        config::clear_active_slot(&db).await.ok();
    }

    let mut write_slot: Option<usize> = None;
    let mut write = WriteState {
        crc: None,
        chunk_count: 0,
        header: None,
    };

    loop {
        let req = STORAGE_REQUEST_CHANNEL.receive().await;
        match req {
            StorageRequest::GetActiveSlot => {
                let slot = config::get_active_slot(&db).await;
                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::ActiveSlot(slot))
                    .await;
            }
            StorageRequest::SetActiveSlot(slot) => {
                let result = if write_slot.is_some() {
                    Err(())
                } else {
                    let mut index = config::get_storage_index(&db).await;
                    index.active_image_id = Some(slot as u32);
                    let result = config::set_storage_index(&db, &index).await;
                    if result.is_ok() {
                        config::set_active_slot(&db, slot).await
                    } else {
                        result
                    }
                };
                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::SetActiveSlot(result))
                    .await;
            }
            StorageRequest::GetSensorConfig => {
                let config = config::get_sensor_config(&db).await;
                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::SensorConfig(config))
                    .await;
            }
            StorageRequest::SetSensorConfig(config) => {
                let result = if write_slot.is_some() {
                    Err(())
                } else {
                    config::set_sensor_config(&db, config).await
                };
                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::SetSensorConfig(result))
                    .await;
            }
            StorageRequest::GetSlotState(slot) => {
                let state = config::get_slot_state(&db, slot).await;
                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::SlotState(state))
                    .await;
            }
            StorageRequest::SetSlotState(slot, state) => {
                let result = if write_slot.is_some() {
                    Err(())
                } else {
                    config::set_slot_state(&db, slot, state).await
                };
                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::SetSlotState(result))
                    .await;
            }
            StorageRequest::ReadSlotData(slot) => {
                let result = image_file::read_slot_data(&db, slot).await;
                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::ReadSlotData(result))
                    .await;
            }
            StorageRequest::ListImageIds => {
                let index = config::get_storage_index(&db).await;
                let ids: Vec<usize> = index
                    .image_ids_slice()
                    .iter()
                    .map(|id| *id as usize)
                    .collect();
                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::ListImageIds(ids))
                    .await;
            }
            StorageRequest::GetStorageStats => {
                let index = config::get_storage_index(&db).await;
                let stats = StorageStats {
                    total_bytes: total_capacity_bytes,
                    used_bytes: index.used_bytes,
                    free_bytes: free_bytes(total_capacity_bytes, index.used_bytes),
                    image_count: index.image_count as usize,
                    active_image_id: index.active_image_id.map(|id| id as usize),
                };
                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::StorageStats(stats))
                    .await;
            }
            StorageRequest::BeginSlotWrite { expected_bytes } => {
                if let Some(prev_slot) = write_slot.take() {
                    image_file::purge_image(&db, prev_slot as u32, write.chunk_count)
                        .await
                        .ok();
                }
                write.crc = None;
                write.chunk_count = 0;
                write.header = None;

                let est_chunks = expected_bytes.div_ceil(CHUNK_SIZE as u32) as u16;
                let required = estimate_image_footprint(expected_bytes, est_chunks);
                let mut index = config::get_storage_index(&db).await;
                let result =
                    if evict_until_capacity(&db, &mut index, total_capacity_bytes, required)
                        .await
                        .is_err()
                    {
                        Err(())
                    } else {
                        let image_id = index.next_image_id;
                        index.next_image_id = index.next_image_id.saturating_add(1);

                        let meta = SlotMetadata {
                            image_id,
                            state: ImageSlotState::Writing,
                            chunk_count: 0,
                        };

                        if image_file::write_slot_metadata(&db, image_id as usize, &meta)
                            .await
                            .is_err()
                        {
                            Err(())
                        } else {
                            if config::set_storage_index(&db, &index).await.is_err() {
                                Err(())
                            } else {
                                write_slot = Some(image_id as usize);
                                write.crc = Some(Crc32Hasher::new());
                                Ok(image_id as usize)
                            }
                        }
                    };

                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::BeginSlotWrite(result))
                    .await;
            }
            StorageRequest::WriteSlotChunk {
                slot,
                chunk_num,
                data,
            } => {
                let result = if write_slot != Some(slot) {
                    Err(())
                } else {
                    match image_file::write_chunk(&db, slot, chunk_num, &data).await {
                        Ok(()) => {
                            if let Some(ref mut h) = write.crc {
                                h.update(&data);
                            }
                            write.chunk_count = write.chunk_count.saturating_add(1);
                            if chunk_num == 0 && data.len() >= 16 && write.header.is_none() {
                                let mut hdr = [0u8; 16];
                                hdr.copy_from_slice(&data[..16]);
                                write.header = Some(hdr);
                            }
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                };
                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::WriteSlotChunk(result))
                    .await;
            }
            StorageRequest::CommitSlot {
                slot,
                expected_crc32,
                total_bytes,
                kind,
                chunk_count,
            } => {
                let result = if write_slot != Some(slot) {
                    Err(())
                } else {
                    let mut commit = CommitContext {
                        total_capacity_bytes,
                        write: &mut write,
                    };
                    commit_slot_ekv(
                        &db,
                        slot,
                        expected_crc32,
                        total_bytes,
                        kind,
                        chunk_count,
                        &mut commit,
                    )
                    .await
                };

                if result.is_err() {
                    image_file::purge_image(&db, slot as u32, chunk_count)
                        .await
                        .ok();
                }

                write_slot = None;
                write = WriteState {
                    crc: None,
                    chunk_count: 0,
                    header: None,
                };

                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::CommitSlot(result))
                    .await;
            }
            StorageRequest::AbortSlot { slot, chunk_count } => {
                let result = if write_slot == Some(slot) {
                    image_file::purge_image(&db, slot as u32, chunk_count).await
                } else {
                    Err(())
                };
                write_slot = None;
                write = WriteState {
                    crc: None,
                    chunk_count: 0,
                    header: None,
                };
                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::AbortSlot(result))
                    .await;
            }
        }
    }
}

async fn commit_slot_ekv(
    db: &ekv_flash::EkvDatabase,
    slot: usize,
    expected_crc32: u32,
    total_bytes: u32,
    kind: DownloadKind,
    chunk_count: u16,
    context: &mut CommitContext<'_>,
) -> Result<(), ()> {
    let actual_crc = context.write.crc.take().map(|h| h.finalize()).unwrap_or(0);
    if actual_crc != expected_crc32 {
        warn!(
            "storage:commit_slot CRC mismatch image_id={} expected={=u32:#010x} actual={=u32:#010x}",
            slot, expected_crc32, actual_crc
        );
        return Err(());
    }

    let header = match context.write.header.take() {
        Some(h) => h,
        None => {
            warn!("storage:commit_slot missing header image_id={}", slot);
            return Err(());
        }
    };

    if &header[..3] != b"POV" || header[3] != 1 {
        warn!(
            "storage:commit_slot invalid header image_id={} magic={=[u8]:?} version={}",
            slot,
            &header[..3],
            header[3]
        );
        return Err(());
    }

    let encoding = match kind {
        DownloadKind::DisplayImage => match postcard::take_from_bytes::<Encoding>(&header[4..]) {
            Ok((enc, _)) => enc,
            Err(_) => {
                warn!("storage:commit_slot unknown encoding image_id={}", slot);
                return Err(());
            }
        },
        DownloadKind::Video => {
            if video::parse_header(&header).is_err() {
                warn!("storage:commit_slot invalid video header image_id={}", slot);
                return Err(());
            }
            // Video frames carry per-frame image headers. Metadata encoding is
            // not used for playback selection, so keep a stable placeholder.
            Encoding::Rgb888Deflate
        }
        DownloadKind::OtaImage => {
            warn!(
                "storage:commit_slot unexpected OtaImage for image_id={}",
                slot
            );
            return Err(());
        }
    };

    let image_kind = match kind {
        DownloadKind::DisplayImage => ImageKind::Static,
        DownloadKind::Video => ImageKind::Video,
        DownloadKind::OtaImage => {
            warn!(
                "storage:commit_slot unexpected OtaImage for image_id={}",
                slot
            );
            return Err(());
        }
    };

    let meta = SlotMetadata {
        image_id: slot as u32,
        state: ImageSlotState::Valid {
            chunk_count,
            total_bytes,
            kind: image_kind,
            encoding,
        },
        chunk_count,
    };
    image_file::write_slot_metadata(db, slot, &meta).await?;

    let mut index = config::get_storage_index(db).await;
    index.push_newest(slot as u32);
    index.used_bytes = index
        .used_bytes
        .saturating_add(estimate_image_footprint(total_bytes, chunk_count));
    index.active_image_id = Some(slot as u32);

    if index.used_bytes > context.total_capacity_bytes {
        warn!(
            "storage:commit_slot accounting overflow used={} total={}",
            index.used_bytes, context.total_capacity_bytes
        );
    }

    config::set_storage_index(db, &index).await?;
    config::set_active_slot(db, slot).await?;

    info!(
        "storage:commit_slot committed image_id={} total_bytes={} crc32={=u32:#010x} encoding={:?}",
        slot, total_bytes, expected_crc32, encoding
    );
    Ok(())
}

pub fn init(flash: esp_hal::peripherals::FLASH<'static>, spawner: Spawner) {
    spawner.spawn(storage_task(flash).unwrap());
}
