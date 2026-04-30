use alloc::vec::Vec;
use core::ops::Range;

use defmt::{error, info, warn};
use embassy_embedded_hal::adapter::BlockingAsync;
use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use esp_bootloader_esp_idf::partitions;
use esp_storage::FlashStorage;

use self::config::{ConfigStore, ImageSlotState};
use self::image_file::ImageFileStore;

pub mod config;
pub mod image_file;

/// Async flash type used throughout the storage module.
pub type AsyncFlash<'d> = BlockingAsync<FlashStorage<'d>>;

/// Maximum bytes per queue push; kept well below the 4096-byte page limit.
pub const CHUNK_SIZE: usize = 3840;

const DOWNLOADABLE_IMAGE_SLOTS: usize = 2;

enum StorageRequest {
    GetActiveSlot,
    SetActiveSlot(u8),
    GetSlotState(usize),
    SetSlotState(usize, ImageSlotState),
    ReadSlotData(usize),
    WriteSlotData { slot: usize, data: Vec<u8> },
}

enum StorageResponse {
    ActiveSlot(Option<u8>),
    SetActiveSlot(Result<(), ()>),
    SlotState(ImageSlotState),
    SetSlotState(Result<(), ()>),
    ReadSlotData(Result<Vec<u8>, ()>),
    WriteSlotData(Result<u16, ()>),
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

pub async fn write_slot_data(slot: usize, data: &[u8]) -> Result<u16, ()> {
    match rpc(StorageRequest::WriteSlotData {
        slot,
        data: data.to_vec(),
    })
    .await
    {
        StorageResponse::WriteSlotData(result) => result,
        _ => {
            warn!("storage:rpc write_slot_data received unexpected response");
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
                let result = if is_valid_slot(slot) {
                    let mut bytes: Vec<u8> = Vec::new();
                    let store = if slot == 0 {
                        &mut img0_store
                    } else {
                        &mut img1_store
                    };
                    store
                        .read_all(&mut flash, &mut chunk_read_buf, |chunk| {
                            bytes.extend_from_slice(chunk)
                        })
                        .await
                        .map(|_| bytes)
                } else {
                    warn!("storage:read_slot_data invalid slot={}", slot);
                    Err(())
                };

                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::ReadSlotData(result))
                    .await;
            }
            StorageRequest::WriteSlotData { slot, data } => {
                let result = if is_valid_slot(slot) {
                    let store = if slot == 0 {
                        &mut img0_store
                    } else {
                        &mut img1_store
                    };
                    store.write_all(&mut flash, &data).await
                } else {
                    warn!("storage:write_slot_data invalid slot={}", slot);
                    Err(())
                };

                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::WriteSlotData(result))
                    .await;
            }
        }
    }
}

pub fn init(flash: FlashStorage<'static>, spawner: Spawner) {
    spawner
        .spawn(storage_task(flash))
        .expect("failed to spawn storage_task");
}
