use alloc::vec::Vec;

use defmt::{info, warn};
use ekv::ReadError;

use super::{
    CHUNK_SIZE,
    config::{ImageSlotState, SlotMetadata},
    ekv_flash::{EkvDatabase, chunk_key, meta_key},
};

/// Read all image data for `slot` from the ekv database.
///
/// Returns the assembled image bytes if the slot is in `Valid` state, or
/// `Err(())` if the slot is empty, in-progress, or any chunk is missing.
pub async fn read_slot_data(db: &EkvDatabase, slot: usize) -> Result<Vec<u8>, ()> {
    let image_id = slot as u32;
    info!("image_file:read_slot_data start image_id={}", image_id);

    // ── 1. Read slot metadata ─────────────────────────────────────────────────
    let rtx = db.read_transaction().await;
    let mut meta_buf = [0u8; SlotMetadata::MAX_SERIALIZED_LEN];
    let meta = match rtx.read(&meta_key(image_id), &mut meta_buf).await {
        Ok(len) => match SlotMetadata::deserialize(&meta_buf[..len]) {
            Some(m) => m,
            None => {
                warn!(
                    "image_file:read_slot_data corrupt metadata image_id={}",
                    image_id
                );
                return Err(());
            }
        },
        Err(ReadError::KeyNotFound) => {
            warn!("image_file:read_slot_data image_id={} not found", image_id);
            return Err(());
        }
        Err(e) => {
            warn!(
                "image_file:read_slot_data metadata read error image_id={}: {:?}",
                image_id,
                defmt::Debug2Format(&e)
            );
            return Err(());
        }
    };

    let (chunk_count, total_bytes) = match meta.state {
        ImageSlotState::Valid {
            chunk_count,
            total_bytes,
            ..
        } => (chunk_count, total_bytes),
        _ => {
            warn!(
                "image_file:read_slot_data image_id={} not in Valid state",
                image_id
            );
            return Err(());
        }
    };

    // ── 2. Assemble chunks ────────────────────────────────────────────────────
    let mut out: Vec<u8> = alloc::vec![0u8; total_bytes as usize];
    // Re-use a heap chunk buffer to avoid large stack allocations.
    let mut chunk_buf: Vec<u8> = alloc::vec![0u8; CHUNK_SIZE];
    let mut offset = 0usize;

    for i in 0..chunk_count {
        let key = chunk_key(image_id, i);
        let n = match rtx.read(&key, &mut chunk_buf).await {
            Ok(n) => n,
            Err(e) => {
                warn!(
                    "image_file:read_slot_data chunk read error image_id={} chunk={}: {:?}",
                    image_id,
                    i,
                    defmt::Debug2Format(&e)
                );
                return Err(());
            }
        };
        let remaining = total_bytes as usize - offset;
        let to_copy = n.min(remaining);
        out[offset..offset + to_copy].copy_from_slice(&chunk_buf[..to_copy]);
        offset += to_copy;
        info!(
            "image_file:read_slot_data image_id={} chunk={}/{} bytes={}",
            image_id, i, chunk_count, to_copy
        );
    }

    // Drop rtx explicitly so it doesn't inadvertently delay a pending write commit.
    drop(rtx);

    info!(
        "image_file:read_slot_data done image_id={} chunks={} total_bytes={}",
        image_id, chunk_count, total_bytes
    );
    Ok(out)
}

/// Delete all chunk records for `slot` and write an Empty metadata record.
///
/// Called by `BeginSlotWrite` and `AbortSlot` to clean up a slot before
/// (re-)using it.  Uses a single write transaction so the cleanup is atomic.
pub async fn erase_slot(db: &EkvDatabase, slot: usize, old_chunk_count: u16) -> Result<(), ()> {
    let image_id = slot as u32;
    info!(
        "image_file:erase_slot image_id={} old_chunk_count={}",
        image_id, old_chunk_count
    );
    info!(
        "image_file:erase_slot acquiring write_transaction image_id={}",
        image_id
    );
    let mut wtx = db.write_transaction().await;
    info!(
        "image_file:erase_slot write_transaction acquired image_id={}",
        image_id
    );

    // Write Empty metadata first so this transaction stays lexicographically
    // ordered: meta key [0x02,..] before chunk keys [0x03,..].
    let meta = SlotMetadata {
        image_id,
        state: ImageSlotState::Empty,
        chunk_count: 0,
    };
    let mut buf = [0u8; SlotMetadata::MAX_SERIALIZED_LEN];
    let serialized = meta.serialize_to(&mut buf)?;
    info!(
        "image_file:erase_slot writing empty metadata image_id={}",
        image_id
    );
    wtx.write(&meta_key(image_id), serialized)
        .await
        .map_err(|_| {
            warn!(
                "image_file:erase_slot write meta error image_id={}",
                image_id
            );
        })?;
    info!(
        "image_file:erase_slot wrote empty metadata image_id={}",
        image_id
    );

    // Delete all existing chunk records for this slot.
    for i in 0..old_chunk_count {
        info!(
            "image_file:erase_slot deleting chunk slot={} chunk={}/{}",
            image_id, i, old_chunk_count
        );
        wtx.delete(&chunk_key(image_id, i)).await.map_err(|_| {
            warn!(
                "image_file:erase_slot delete chunk error image_id={} chunk={}",
                image_id, i
            );
        })?;
    }
    info!("image_file:erase_slot deleted chunks image_id={}", image_id);

    info!(
        "image_file:erase_slot committing transaction image_id={}",
        image_id
    );
    wtx.commit().await.map_err(|_| {
        warn!("image_file:erase_slot commit error image_id={}", image_id);
    })?;
    info!("image_file:erase_slot commit ok image_id={}", image_id);
    Ok(())
}

/// Fully remove an image metadata record and all chunk keys.
pub async fn purge_image(db: &EkvDatabase, image_id: u32, chunk_count: u16) -> Result<(), ()> {
    let mut wtx = db.write_transaction().await;
    // Keep transaction mutations sorted by key: metadata key [0x02,..] first.
    wtx.delete(&meta_key(image_id)).await.map_err(|_| {
        warn!(
            "image_file:purge_image delete metadata error image_id={}",
            image_id
        );
    })?;
    for chunk_num in 0..chunk_count {
        wtx.delete(&chunk_key(image_id, chunk_num))
            .await
            .map_err(|_| {
                warn!(
                    "image_file:purge_image delete chunk error image_id={} chunk={}",
                    image_id, chunk_num
                );
            })?;
    }
    wtx.commit().await.map_err(|_| {
        warn!("image_file:purge_image commit error image_id={}", image_id);
    })
}

/// Write a single image chunk into ekv using its own committed write transaction.
///
/// Each chunk is an independent atomic write.  If power is lost mid-download,
/// only the chunks that were fully committed survive; the slot will not be
/// marked Valid (that only happens in `write_slot_metadata`) so it is treated
/// as Empty on the next boot.
pub async fn write_chunk(
    db: &EkvDatabase,
    slot: usize,
    chunk_num: u16,
    data: &[u8],
) -> Result<(), ()> {
    let image_id = slot as u32;
    info!(
        "image_file:write_chunk start image_id={} chunk={} bytes={}",
        image_id,
        chunk_num,
        data.len()
    );
    info!(
        "image_file:write_chunk acquiring write_transaction image_id={} chunk={}",
        image_id, chunk_num
    );
    let mut wtx = db.write_transaction().await;
    info!(
        "image_file:write_chunk write_transaction acquired image_id={} chunk={}",
        image_id, chunk_num
    );
    info!(
        "image_file:write_chunk writing chunk image_id={} chunk={} bytes={}",
        image_id,
        chunk_num,
        data.len()
    );
    wtx.write(&chunk_key(image_id, chunk_num), data)
        .await
        .map_err(|_| {
            warn!(
                "image_file:write_chunk write error image_id={} chunk={}",
                image_id, chunk_num
            );
        })?;
    info!(
        "image_file:write_chunk wrote chunk image_id={} chunk={}",
        image_id, chunk_num
    );
    info!(
        "image_file:write_chunk committing image_id={} chunk={}",
        image_id, chunk_num
    );
    wtx.commit().await.map_err(|_| {
        warn!(
            "image_file:write_chunk commit error image_id={} chunk={}",
            image_id, chunk_num
        );
    })?;
    info!(
        "image_file:write_chunk commit ok image_id={} chunk={}",
        image_id, chunk_num
    );
    Ok(())
}

/// Atomically write the final Valid metadata for a slot.
///
/// This is the single commit that makes the slot readable.  All chunk writes
/// must be done (via `write_chunk`) before calling this.
pub async fn write_slot_metadata(
    db: &EkvDatabase,
    slot: usize,
    meta: &SlotMetadata,
) -> Result<(), ()> {
    let image_id = slot as u32;
    info!("image_file:write_slot_metadata start image_id={}", image_id);
    let mut buf = [0u8; SlotMetadata::MAX_SERIALIZED_LEN];
    let serialized = meta.serialize_to(&mut buf)?;

    info!(
        "image_file:write_slot_metadata acquiring write_transaction slot={}",
        image_id
    );
    let mut wtx = db.write_transaction().await;
    info!(
        "image_file:write_slot_metadata write_transaction acquired slot={}",
        image_id
    );
    info!(
        "image_file:write_slot_metadata writing metadata slot={}",
        image_id
    );
    wtx.write(&meta_key(image_id), serialized)
        .await
        .map_err(|_| {
            warn!(
                "image_file:write_slot_metadata write error image_id={}",
                image_id
            );
        })?;
    info!(
        "image_file:write_slot_metadata wrote metadata slot={}",
        image_id
    );
    info!(
        "image_file:write_slot_metadata committing image_id={}",
        image_id
    );
    wtx.commit().await.map_err(|_| {
        warn!(
            "image_file:write_slot_metadata commit error image_id={}",
            image_id
        );
    })?;
    info!(
        "image_file:write_slot_metadata commit ok image_id={}",
        image_id
    );
    Ok(())
}
