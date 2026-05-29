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

use self::config::{ImageKind, ImageSlotState, SlotMetadata};
use self::ekv_flash::EkvFlash;

pub mod config;
pub mod ekv_flash;
pub mod image_file;

/// Maximum bytes per image chunk (used for writes and CRC accumulation).
pub const CHUNK_SIZE: usize = 3840;

const DOWNLOADABLE_IMAGE_SLOTS: usize = 2;

enum StorageRequest {
    GetActiveSlot,
    SetActiveSlot(u8),
    GetSlotState(usize),
    SetSlotState(usize, ImageSlotState),
    ReadSlotData(usize),
    // ---- streaming write API ----
    /// Begin a new streaming write: clean up the chosen slot and return its index.
    BeginSlotWrite,
    /// Write a single chunk of image data identified by chunk number.
    WriteSlotChunk {
        slot: usize,
        chunk_num: u16,
        data: Vec<u8>,
    },
    /// Verify the in-memory CRC and commit the slot as Valid.
    CommitSlot {
        slot: usize,
        expected_crc32: u32,
        total_bytes: u32,
        kind: DownloadKind,
    },
    /// Discard any in-progress write for this slot, marking it Empty.
    AbortSlot {
        slot: usize,
    },
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

/// Write a single image chunk identified by `chunk_num` within the given slot.
///
/// `chunk_num` is computed by the caller as `byte_offset / CHUNK_SIZE`.
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
pub async fn storage_task(flash: esp_hal::peripherals::FLASH<'static>) -> ! {
    info!("storage:task started");

    // On dual-core builds, use auto-park while render tasks cooperate via
    // Core1 tasks cooperate via `pause_render_for_flash` which drives them into
    // IRAM-resident spin loops before any flash mutation begins.  Once in the IRAM
    // spin they fetch no instructions from flash-backed ICache pages, so
    // Cache_Disable_ICache (called by ROM flash routines) is safe.  We use
    // `multicore_ignore` here because the coordination is handled externally by
    // the caller via `led::pause_render_for_flash` / `resume_render_after_flash`.
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
    info!(
        "storage:task pov_store={:#x}..{:#x}",
        pov_store_range.start, pov_store_range.end
    );

    let ekv_flash = EkvFlash::new(
        flash_storage,
        pov_store_range.start,
        pov_store_range.end - pov_store_range.start,
    );
    let db: ekv_flash::EkvDatabase = Database::new(ekv_flash, Config::default());

    match db.mount().await {
        Ok(()) => info!("storage:task ekv mounted"),
        Err(MountError::Corrupted) => {
            warn!("storage:task ekv not formatted or corrupted, formatting...");
            db.format().await.expect("storage:task ekv format failed");
            db.mount().await.expect("storage:task ekv re-mount failed");
            info!("storage:task ekv formatted and mounted");
        }
        Err(e) => {
            error!(
                "storage:task ekv mount error: {:?}",
                defmt::Debug2Format(&e)
            );
            panic!()
        }
    }

    // ── In-flight write state (lives only in RAM, never persisted mid-write) ──
    let mut write_slot: Option<usize> = None;
    let mut write_crc: Option<Crc32Hasher> = None;
    let mut write_chunk_count: u16 = 0;
    let mut write_header: Option<[u8; 16]> = None;

    loop {
        let req = STORAGE_REQUEST_CHANNEL.receive().await;
        match req {
            // ── Config queries ────────────────────────────────────────────────
            StorageRequest::GetActiveSlot => {
                let slot = config::get_active_slot(&db).await;
                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::ActiveSlot(slot))
                    .await;
            }
            StorageRequest::SetActiveSlot(slot) => {
                let result = if write_slot.is_some() {
                    warn!("storage:set_active_slot rejected: write in progress");
                    Err(())
                } else {
                    config::set_active_slot(&db, slot).await
                };
                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::SetActiveSlot(result))
                    .await;
            }
            StorageRequest::GetSlotState(slot) => {
                let state = if is_valid_slot(slot) {
                    config::get_slot_state(&db, slot).await
                } else {
                    warn!("storage:get_slot_state invalid slot={}", slot);
                    ImageSlotState::Empty
                };
                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::SlotState(state))
                    .await;
            }
            StorageRequest::SetSlotState(slot, state) => {
                let result = if write_slot.is_some() {
                    warn!("storage:set_slot_state rejected: write in progress");
                    Err(())
                } else if is_valid_slot(slot) {
                    config::set_slot_state(&db, slot, state).await
                } else {
                    warn!("storage:set_slot_state invalid slot={}", slot);
                    Err(())
                };
                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::SetSlotState(result))
                    .await;
            }
            // ── Image data read ───────────────────────────────────────────────
            StorageRequest::ReadSlotData(slot) => {
                let result = if is_valid_slot(slot) {
                    image_file::read_slot_data(&db, slot).await
                } else {
                    warn!("storage:read_slot_data invalid slot={}", slot);
                    Err(())
                };
                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::ReadSlotData(result))
                    .await;
            }
            // ── Streaming write ───────────────────────────────────────────────
            StorageRequest::BeginSlotWrite => {
                info!("storage:begin_slot_write request received");
                // Clean up any previous in-progress write.
                if let Some(prev_slot) = write_slot.take() {
                    info!(
                        "storage:begin_slot_write cleaning up abandoned slot={}",
                        prev_slot
                    );
                    image_file::erase_slot(&db, prev_slot, write_chunk_count)
                        .await
                        .ok();
                }
                write_crc = None;
                write_chunk_count = 0;
                write_header = None;

                // Pick the slot that is NOT currently active.
                info!("storage:begin_slot_write reading active slot");
                let active = config::get_active_slot(&db).await;
                info!("storage:begin_slot_write active slot={:?}", active);
                let slot = match active {
                    Some(a) => (a as usize + 1) % DOWNLOADABLE_IMAGE_SLOTS,
                    None => 0,
                };
                info!("storage:begin_slot_write slot={}", slot);

                // Erase the chosen slot's existing data.
                info!("storage:begin_slot_write reading slot state slot={}", slot);
                let old_chunk_count = match config::get_slot_state(&db, slot).await {
                    ImageSlotState::Valid { chunk_count, .. } => chunk_count,
                    _ => 0,
                };
                info!(
                    "storage:begin_slot_write erase start slot={} old_chunk_count={}",
                    slot, old_chunk_count
                );
                let result = image_file::erase_slot(&db, slot, old_chunk_count)
                    .await
                    .map(|_| {
                        info!("storage:begin_slot_write erase ok slot={}", slot);
                        write_slot = Some(slot);
                        write_crc = Some(Crc32Hasher::new());
                        slot
                    });

                if result.is_err() {
                    warn!("storage:begin_slot_write erase failed slot={}", slot);
                }
                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::BeginSlotWrite(result))
                    .await;
            }
            StorageRequest::WriteSlotChunk {
                slot,
                chunk_num,
                data,
            } => {
                info!(
                    "storage:write_slot_chunk request slot={} chunk={} bytes={}",
                    slot,
                    chunk_num,
                    data.len()
                );
                let result = if write_slot != Some(slot) {
                    warn!(
                        "storage:write_slot_chunk slot mismatch: expected {:?} got {}",
                        write_slot, slot
                    );
                    Err(())
                } else if !is_valid_slot(slot) {
                    warn!("storage:write_slot_chunk invalid slot={}", slot);
                    Err(())
                } else {
                    // Write this chunk via its own committed transaction.
                    info!(
                        "storage:write_slot_chunk dispatch write slot={} chunk={}",
                        slot, chunk_num
                    );
                    match image_file::write_chunk(&db, slot, chunk_num, &data).await {
                        Ok(()) => {
                            info!(
                                "storage:write_slot_chunk write ok slot={} chunk={}",
                                slot, chunk_num
                            );
                            // Accumulate CRC in RAM.
                            if let Some(ref mut h) = write_crc {
                                h.update(&data);
                            }
                            write_chunk_count += 1;
                            // Capture the image header from the very first chunk.
                            if chunk_num == 0 && data.len() >= 16 && write_header.is_none() {
                                let mut hdr = [0u8; 16];
                                hdr.copy_from_slice(&data[..16]);
                                write_header = Some(hdr);
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
            } => {
                info!(
                    "storage:commit_slot request slot={} expected_crc32={=u32:#010x} total_bytes={} kind={:?}",
                    slot, expected_crc32, total_bytes, kind
                );
                let result = if write_slot != Some(slot) {
                    warn!(
                        "storage:commit_slot slot mismatch: expected {:?} got {}",
                        write_slot, slot
                    );
                    Err(())
                } else {
                    commit_slot_ekv(
                        &db,
                        slot,
                        expected_crc32,
                        total_bytes,
                        kind,
                        &mut write_crc,
                        &mut write_header,
                    )
                    .await
                };

                if result.is_ok() {
                    info!("storage:commit_slot result ok slot={}", slot);
                } else {
                    warn!("storage:commit_slot result err slot={}", slot);
                }

                // Always clear write state after commit attempt.
                write_slot = None;
                write_crc = None;
                write_chunk_count = 0;
                write_header = None;

                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::CommitSlot(result))
                    .await;
            }
            StorageRequest::AbortSlot { slot } => {
                let result = if write_slot == Some(slot) {
                    info!("storage:abort_slot slot={}", slot);
                    let r = image_file::erase_slot(&db, slot, write_chunk_count).await;
                    write_slot = None;
                    write_crc = None;
                    write_chunk_count = 0;
                    write_header = None;
                    r
                } else {
                    warn!(
                        "storage:abort_slot slot mismatch: expected {:?} got {}",
                        write_slot, slot
                    );
                    Err(())
                };
                STORAGE_RESPONSE_CHANNEL
                    .send(StorageResponse::AbortSlot(result))
                    .await;
            }
        }
    }
}

/// Verify the in-memory CRC, parse the image header, and atomically commit the
/// slot as `Valid` in the ekv database.
///
/// On any failure the slot metadata is written as `Empty` and `Err(())` is returned.
async fn commit_slot_ekv(
    db: &ekv_flash::EkvDatabase,
    slot: usize,
    expected_crc32: u32,
    total_bytes: u32,
    kind: DownloadKind,
    write_crc: &mut Option<Crc32Hasher>,
    write_header: &mut Option<[u8; 16]>,
) -> Result<(), ()> {
    // 1. Verify CRC accumulated in RAM.
    let actual_crc = write_crc.take().map(|h| h.finalize()).unwrap_or(0);
    if actual_crc != expected_crc32 {
        warn!(
            "storage:commit_slot CRC mismatch slot={} expected={=u32:#010x} actual={=u32:#010x}",
            slot, expected_crc32, actual_crc
        );
        mark_slot_empty(db, slot).await;
        return Err(());
    }

    // 2. Validate image header from first chunk (captured in RAM).
    let header = match write_header.take() {
        Some(h) => h,
        None => {
            warn!("storage:commit_slot missing header slot={}", slot);
            mark_slot_empty(db, slot).await;
            return Err(());
        }
    };

    if &header[..3] != b"POV" || header[3] != 1 {
        warn!(
            "storage:commit_slot invalid header slot={} magic={=[u8]:?} version={}",
            slot,
            &header[..3],
            header[3]
        );
        mark_slot_empty(db, slot).await;
        return Err(());
    }

    let encoding = match postcard::take_from_bytes::<Encoding>(&header[4..]) {
        Ok((enc, _)) => enc,
        Err(_) => {
            warn!(
                "storage:commit_slot unknown encoding slot={} header={=[u8]:?}",
                slot,
                &header[4..8]
            );
            mark_slot_empty(db, slot).await;
            return Err(());
        }
    };

    // 3. Map DownloadKind → ImageKind.
    let image_kind = match kind {
        DownloadKind::DisplayImage => ImageKind::Static,
        DownloadKind::Video => ImageKind::Video,
        DownloadKind::OtaImage => {
            warn!(
                "storage:commit_slot unexpected OtaImage kind for image slot={}",
                slot
            );
            mark_slot_empty(db, slot).await;
            return Err(());
        }
    };

    let chunk_count = total_bytes.div_ceil(CHUNK_SIZE as u32) as u16;

    // 4. Atomically write Valid metadata.
    let meta = SlotMetadata {
        state: ImageSlotState::Valid {
            chunk_count,
            total_bytes,
            kind: image_kind,
            encoding,
        },
        chunk_count,
        base_key: slot as u8,
    };
    image_file::write_slot_metadata(db, slot, &meta).await?;

    // 5. Update active slot pointer.
    config::set_active_slot(db, slot as u8).await?;

    info!(
        "storage:commit_slot committed slot={} total_bytes={} crc32={=u32:#010x} encoding={:?}",
        slot, total_bytes, expected_crc32, encoding
    );
    Ok(())
}

/// Write Empty metadata for `slot` (best-effort; errors logged but not propagated).
async fn mark_slot_empty(db: &ekv_flash::EkvDatabase, slot: usize) {
    let meta = SlotMetadata {
        state: ImageSlotState::Empty,
        chunk_count: 0,
        base_key: slot as u8,
    };
    if image_file::write_slot_metadata(db, slot, &meta)
        .await
        .is_err()
    {
        warn!("storage:mark_slot_empty write failed slot={}", slot);
    }
}

pub fn init(flash: esp_hal::peripherals::FLASH<'static>, spawner: Spawner) {
    spawner.spawn(storage_task(flash).unwrap());
}
