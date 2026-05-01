use alloc::vec::Vec;
use core::ops::Range;

use defmt::{error, info, warn};
use embassy_embedded_hal::adapter::BlockingAsync;
use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use esp_bootloader_esp_idf::partitions;
use esp_storage::FlashStorage;
use pov_proto::image::Encoding;
use pov_proto::transfer::DownloadKind;

use self::config::{ConfigStore, ImageKind, ImageSlotState};
use self::image_file::ImageFileStore;

pub mod config;
pub mod image_file;

/// Async flash type used throughout the storage module.
pub type AsyncFlash<'d> = BlockingAsync<FlashStorage<'d>>;

/// Maximum bytes per raw flash write block (used for reads/CRC verification).
pub const CHUNK_SIZE: usize = 3840;

const DOWNLOADABLE_IMAGE_SLOTS: usize = 2;

enum StorageRequest {
    GetActiveSlot,
    SetActiveSlot(u8),
    GetSlotState(usize),
    SetSlotState(usize, ImageSlotState),
    ReadSlotData(usize),
    // ---- streaming write API ----
    /// Begin a new streaming write: erase the chosen slot and return its index.
    BeginSlotWrite,
    /// Write a chunk of image data at the given byte offset within the slot.
    WriteSlotChunk {
        slot: usize,
        offset: u32,
        data: Vec<u8>,
    },
    /// Verify the CRC of the written data and commit the slot as Valid.
    CommitSlot {
        slot: usize,
        expected_crc32: u32,
        total_bytes: u32,
        kind: DownloadKind,
    },
    /// Mark the slot as Empty, discarding any in-progress write.
    AbortSlot { slot: usize },
}

enum StorageResponse {
    ActiveSlot(Option<u8>),
    SetActiveSlot(Result<(), ()>),
    SlotState(ImageSlotState),
    SetSlotState(Result<(), ()>),
    ReadSlotData(Result<Vec<u8>, ()>),
    // ---- streaming write API ----
    /// Returns the slot index allocated for this write, or Err on failure.
    BeginSlotWrite(Result<usize, ()>),
    WriteSlotChunk(Result<(), ()>),
    CommitSlot(Result<(), ()>),
    AbortSlot(Result<(), ()>),
}

static STORAGE_REQUEST_CHANNEL: Channel<CriticalSectionRawMutex, StorageRequest, 4> =
    Channel::new();
static STORAGE_RESPONSE_CHANNEL: Channel<CriticalSectionRawMutex, StorageResponse, 4> =
    Channel::new();

fn is_valid_slot(slot: usize) -> bool {
    slot < DOWNLOADABLE_IMAGE_SLOTS
}

async fn rpc(req: StorageRequest) -> StorageResponse {
    STORAGE_REQUEST_CHANNEL.send(req).await;
    STORAGE_RESPONSE_CHANNEL.receive().await
}

pub async fn get_active_slot() -> Option<u8> {
    match rpc(StorageRequest::GetActiveSlot).await {
        StorageResponse::ActiveSlot(slot) => slot,
        _ => {
            warn!("storage:rpc get_active_slot received unexpected response");
            None
        }
    }
}

pub async fn set_active_slot(slot: u8) -> Result<(), ()> {
    match rpc(StorageRequest::SetActiveSlot(slot)).await {
        StorageResponse::SetActiveSlot(result) => result,
        _ => {
            warn!("storage:rpc set_active_slot received unexpected response");
            Err(())
        }
    }
}

pub async fn get_slot_state(slot: usize) -> ImageSlotState {
    match rpc(StorageRequest::GetSlotState(slot)).await {
        StorageResponse::SlotState(state) => state,
        _ => {
            warn!("storage:rpc get_slot_state received unexpected response");
            ImageSlotState::Empty
        }
    }
}

pub async fn set_slot_state(slot: usize, state: ImageSlotState) -> Result<(), ()> {
    match rpc(StorageRequest::SetSlotState(slot, state)).await {
        StorageResponse::SetSlotState(result) => result,
        _ => {
            warn!("storage:rpc set_slot_state received unexpected response");
            Err(())
        }
    }
}

pub async fn read_slot_data(slot: usize) -> Result<Vec<u8>, ()> {
    match rpc(StorageRequest::ReadSlotData(slot)).await {
        StorageResponse::ReadSlotData(result) => result,
        _ => {
            warn!("storage:rpc read_slot_data received unexpected response");
            Err(())
        }
    }
}

/// Allocate a flash slot for a new streaming write.
///
/// The storage task picks the slot that is not currently active, erases it,
/// and marks it as `Writing`.  Returns the slot index on success.
pub async fn begin_slot_write() -> Result<usize, ()> {
    match rpc(StorageRequest::BeginSlotWrite).await {
        StorageResponse::BeginSlotWrite(result) => result,
        _ => {
            warn!("storage:rpc begin_slot_write received unexpected response");
            Err(())
        }
    }
}

/// Write a chunk of image data at `offset` within the given slot.
pub async fn write_slot_chunk(slot: usize, offset: u32, data: &[u8]) -> Result<(), ()> {
    match rpc(StorageRequest::WriteSlotChunk {
        slot,
        offset,
        data: data.to_vec(),
    })
    .await
    {
        StorageResponse::WriteSlotChunk(result) => result,
        _ => {
            warn!("storage:rpc write_slot_chunk received unexpected response");
            Err(())
        }
    }
}

/// Verify the CRC of the written data and, if it matches, commit the slot.
///
/// On success the slot is marked `Valid` and becomes the active slot.
/// On failure (CRC mismatch or I/O error) the slot is marked `Empty`.
pub async fn commit_slot(
    slot: usize,
    expected_crc32: u32,
    total_bytes: u32,
    kind: DownloadKind,
) -> Result<(), ()> {
    match rpc(StorageRequest::CommitSlot {
        slot,
        expected_crc32,
        total_bytes,
        kind,
    })
    .await
    {
        StorageResponse::CommitSlot(result) => result,
        _ => {
            warn!("storage:rpc commit_slot received unexpected response");
            Err(())
        }
    }
}

/// Abort an in-progress slot write, marking it `Empty`.
pub async fn abort_slot(slot: usize) -> Result<(), ()> {
    match rpc(StorageRequest::AbortSlot { slot }).await {
        StorageResponse::AbortSlot(result) => result,
        _ => {
            warn!("storage:rpc abort_slot received unexpected response");
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

#[embassy_executor::task]
pub async fn storage_task(mut flash: FlashStorage<'static>) -> ! {
    info!("storage:task started");

    let mut partition_table_raw = [0u8; partitions::PARTITION_TABLE_MAX_LEN];
    let partition_table = partitions::read_partition_table(&mut flash, &mut partition_table_raw)
        .expect("storage:task failed to read partition table");

    let config_range = find_partition_range(&partition_table, "pov_config").unwrap_or_else(|| {
        error!("storage:task partition 'pov_config' not found");
        panic!()
    });
    let img0_range = find_partition_range(&partition_table, "pov_img_0").unwrap_or_else(|| {
        error!("storage:task partition 'pov_img_0' not found");
        panic!()
    });
    let img1_range = find_partition_range(&partition_table, "pov_img_1").unwrap_or_else(|| {
        error!("storage:task partition 'pov_img_1' not found");
        panic!()
    });

    info!(
        "storage:task partitions: config={:#x}..{:#x} img0={:#x}..{:#x} img1={:#x}..{:#x}",
        config_range.start,
        config_range.end,
        img0_range.start,
        img0_range.end,
        img1_range.start,
        img1_range.end
    );

    let mut flash = BlockingAsync::new(flash);

    let mut config_store = ConfigStore::new(config_range);
    let mut img0_store = ImageFileStore::new(0, img0_range);
    let mut img1_store = ImageFileStore::new(1, img1_range);

    let mut config_scratch = [0u8; 256];
    let mut chunk_read_buf = [0u8; CHUNK_SIZE];

    loop {
        let req = STORAGE_REQUEST_CHANNEL.receive().await;
        match req {
            StorageRequest::GetActiveSlot => {
                let slot = config_store
                    .get_active_slot(&mut flash, &mut config_scratch)
                    .await;
                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::ActiveSlot(slot))
                    .await;
            }
            StorageRequest::SetActiveSlot(slot) => {
                let result = config_store
                    .set_active_slot(&mut flash, slot, &mut config_scratch)
                    .await;
                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::SetActiveSlot(result))
                    .await;
            }
            StorageRequest::GetSlotState(slot) => {
                let state = if is_valid_slot(slot) {
                    config_store
                        .get_slot_state(&mut flash, slot, &mut config_scratch)
                        .await
                } else {
                    warn!("storage:get_slot_state invalid slot={}", slot);
                    ImageSlotState::Empty
                };
                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::SlotState(state))
                    .await;
            }
            StorageRequest::SetSlotState(slot, state) => {
                let result = if is_valid_slot(slot) {
                    config_store
                        .set_slot_state(&mut flash, slot, &state, &mut config_scratch)
                        .await
                } else {
                    warn!("storage:set_slot_state invalid slot={}", slot);
                    Err(())
                };
                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::SetSlotState(result))
                    .await;
            }
            StorageRequest::ReadSlotData(slot) => {
                // Read the raw image bytes written by the streaming write path.
                let result = if is_valid_slot(slot) {
                    let state = config_store
                        .get_slot_state(&mut flash, slot, &mut config_scratch)
                        .await;
                    if let ImageSlotState::Valid { total_bytes, .. } = state {
                        let aligned = ((total_bytes as usize) + 3) & !3;
                        let mut bytes: Vec<u8> = alloc::vec![0u8; aligned];
                        let store = if slot == 0 {
                            &mut img0_store
                        } else {
                            &mut img1_store
                        };
                        store
                            .read_raw(&mut flash, total_bytes, &mut bytes)
                            .await
                            .map(|_| {
                                bytes.truncate(total_bytes as usize);
                                bytes
                            })
                    } else {
                        warn!(
                            "storage:read_slot_data slot={} not in Valid state",
                            slot
                        );
                        Err(())
                    }
                } else {
                    warn!("storage:read_slot_data invalid slot={}", slot);
                    Err(())
                };

                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::ReadSlotData(result))
                    .await;
            }
            StorageRequest::BeginSlotWrite => {
                // Pick the slot that is NOT currently active, so we don't
                // clobber the image that is still being displayed.
                let active = config_store
                    .get_active_slot(&mut flash, &mut config_scratch)
                    .await;
                let slot = match active {
                    Some(a) => (a as usize + 1) % DOWNLOADABLE_IMAGE_SLOTS,
                    None => 0,
                };
                info!("storage:begin_slot_write slot={}", slot);

                let result = async {
                    config_store
                        .set_slot_state(
                            &mut flash,
                            slot,
                            &ImageSlotState::Writing,
                            &mut config_scratch,
                        )
                        .await?;
                    let store = if slot == 0 {
                        &mut img0_store
                    } else {
                        &mut img1_store
                    };
                    store.erase_for_streaming(&mut flash).await?;
                    Ok(slot)
                }
                .await;

                if result.is_err() {
                    warn!("storage:begin_slot_write failed slot={}", slot);
                }
                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::BeginSlotWrite(result))
                    .await;
            }
            StorageRequest::WriteSlotChunk { slot, offset, data } => {
                let result = if is_valid_slot(slot) {
                    let store = if slot == 0 {
                        &mut img0_store
                    } else {
                        &mut img1_store
                    };
                    store.write_at_offset(&mut flash, offset, &data).await
                } else {
                    warn!("storage:write_slot_chunk invalid slot={}", slot);
                    Err(())
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
            } => {
                let result = if is_valid_slot(slot) {
                    commit_slot_inner(
                        slot,
                        expected_crc32,
                        total_bytes,
                        kind,
                        &mut flash,
                        &mut config_store,
                        &mut img0_store,
                        &mut img1_store,
                        &mut config_scratch,
                        &mut chunk_read_buf,
                    )
                    .await
                } else {
                    warn!("storage:commit_slot invalid slot={}", slot);
                    Err(())
                };
                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::CommitSlot(result))
                    .await;
            }
            StorageRequest::AbortSlot { slot } => {
                let result = if is_valid_slot(slot) {
                    info!("storage:abort_slot slot={}", slot);
                    config_store
                        .set_slot_state(
                            &mut flash,
                            slot,
                            &ImageSlotState::Empty,
                            &mut config_scratch,
                        )
                        .await
                } else {
                    warn!("storage:abort_slot invalid slot={}", slot);
                    Err(())
                };
                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::AbortSlot(result))
                    .await;
            }
        }
    }
}

/// Verify CRC, parse the image header, and commit the slot as Valid.
///
/// On failure the slot is marked `Empty` and `Err(())` is returned.
#[allow(clippy::too_many_arguments)]
async fn commit_slot_inner(
    slot: usize,
    expected_crc32: u32,
    total_bytes: u32,
    kind: DownloadKind,
    flash: &mut AsyncFlash<'_>,
    config_store: &mut ConfigStore,
    img0_store: &mut ImageFileStore,
    img1_store: &mut ImageFileStore,
    config_scratch: &mut [u8],
    chunk_read_buf: &mut [u8],
) -> Result<(), ()> {
    let store = if slot == 0 { img0_store } else { img1_store };

    // 1. Verify CRC by reading back the data from flash.
    info!(
        "storage:commit_slot verifying CRC slot={} total_bytes={} expected_crc32={=u32:#010x}",
        slot, total_bytes, expected_crc32
    );
    if let Err(()) = store
        .verify_crc(flash, total_bytes, expected_crc32, chunk_read_buf)
        .await
    {
        warn!("storage:commit_slot CRC mismatch, aborting slot={}", slot);
        config_store
            .set_slot_state(flash, slot, &ImageSlotState::Empty, config_scratch)
            .await
            .ok();
        return Err(());
    }

    // 2. Read the image header to extract the encoding.
    //    Header layout: magic[3] + version[1] + encoding[1] = 5 bytes.
    //    Read 8 bytes (word-aligned) and inspect the first 5.
    let mut header_buf = [0u8; 8];
    if store
        .read_raw(flash, 8, &mut header_buf)
        .await
        .is_err()
    {
        warn!("storage:commit_slot header read failed slot={}", slot);
        config_store
            .set_slot_state(flash, slot, &ImageSlotState::Empty, config_scratch)
            .await
            .ok();
        return Err(());
    }

    let magic = &header_buf[..3];
    let version = header_buf[3];
    let encoding_byte = header_buf[4];

    if magic != b"POV" || version != 1 {
        warn!(
            "storage:commit_slot invalid image header slot={} magic={=[u8]:?} version={}",
            slot, magic, version
        );
        config_store
            .set_slot_state(flash, slot, &ImageSlotState::Empty, config_scratch)
            .await
            .ok();
        return Err(());
    }

    let encoding = postcard::from_bytes::<Encoding>(&[encoding_byte]).map_err(|_| {
        warn!(
            "storage:commit_slot unknown encoding byte={} slot={}",
            encoding_byte, slot
        );
    })?;

    // 3. Map DownloadKind → ImageKind.
    let image_kind = match kind {
        DownloadKind::DisplayImage => ImageKind::Static,
        DownloadKind::Video => ImageKind::Video,
        DownloadKind::OtaImage => {
            warn!("storage:commit_slot unexpected OtaImage kind for image slot");
            config_store
                .set_slot_state(flash, slot, &ImageSlotState::Empty, config_scratch)
                .await
                .ok();
            return Err(());
        }
    };

    let chunk_count = (total_bytes as u32).div_ceil(CHUNK_SIZE as u32) as u16;

    // 4. Persist the Valid state and update the active slot pointer.
    let state = ImageSlotState::Valid {
        chunk_count,
        total_bytes,
        kind: image_kind,
        encoding,
    };
    config_store
        .set_slot_state(flash, slot, &state, config_scratch)
        .await?;
    config_store
        .set_active_slot(flash, slot as u8, config_scratch)
        .await?;

    info!(
        "storage:commit_slot committed slot={} total_bytes={} crc32={=u32:#010x} encoding={:?}",
        slot, total_bytes, expected_crc32, encoding
    );
    Ok(())
}

pub fn init(flash: FlashStorage<'static>, spawner: Spawner) {
    spawner
        .spawn(storage_task(flash))
        .expect("failed to spawn storage_task");
}
