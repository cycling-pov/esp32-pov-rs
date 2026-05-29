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
    info!("image_file:read_slot_data start slot={}", slot);

    // ── 1. Read slot metadata ─────────────────────────────────────────────────
    let rtx = db.read_transaction().await;
    let mut meta_buf = [0u8; SlotMetadata::MAX_SERIALIZED_LEN];
    let meta = match rtx.read(&meta_key(slot), &mut meta_buf).await {
        Ok(len) => match SlotMetadata::deserialize(&meta_buf[..len]) {
            Some(m) => m,
            None => {
                warn!("image_file:read_slot_data corrupt metadata slot={}", slot);
                return Err(());
            }
        },
        Err(ReadError::KeyNotFound) => {
            warn!("image_file:read_slot_data slot={} not found", slot);
            return Err(());
        }
        Err(e) => {
            warn!(
                "image_file:read_slot_data metadata read error slot={}: {:?}",
                slot,
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
            warn!("image_file:read_slot_data slot={} not in Valid state", slot);
            return Err(());
        }
    };

    // ── 2. Assemble chunks ────────────────────────────────────────────────────
    let mut out: Vec<u8> = alloc::vec![0u8; total_bytes as usize];
    // Re-use a heap chunk buffer to avoid large stack allocations.
    let mut chunk_buf: Vec<u8> = alloc::vec![0u8; CHUNK_SIZE];
    let mut offset = 0usize;

    for i in 0..chunk_count {
        let key = chunk_key(slot, i);
        let n = match rtx.read(&key, &mut chunk_buf).await {
            Ok(n) => n,
            Err(e) => {
                warn!(
                    "image_file:read_slot_data chunk read error slot={} chunk={}: {:?}",
                    slot,
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
            "image_file:read_slot_data slot={} chunk={}/{} bytes={}",
            slot, i, chunk_count, to_copy
        );
    }

    // Drop rtx explicitly so it doesn't inadvertently delay a pending write commit.
    drop(rtx);

    info!(
        "image_file:read_slot_data done slot={} chunks={} total_bytes={}",
        slot, chunk_count, total_bytes
    );
    Ok(out)
}

/// Delete all chunk records for `slot` and write an Empty metadata record.
///
/// Called by `BeginSlotWrite` and `AbortSlot` to clean up a slot before
/// (re-)using it.  Uses a single write transaction so the cleanup is atomic.
pub async fn erase_slot(db: &EkvDatabase, slot: usize, old_chunk_count: u16) -> Result<(), ()> {
    info!(
        "image_file:erase_slot slot={} old_chunk_count={}",
        slot, old_chunk_count
    );
    info!(
        "image_file:erase_slot acquiring write_transaction slot={}",
        slot
    );
    let mut wtx = db.write_transaction().await;
    info!(
        "image_file:erase_slot write_transaction acquired slot={}",
        slot
    );

    // Write Empty metadata first so this transaction stays lexicographically
    // ordered: meta key [0x02,..] before chunk keys [0x03,..].
    let meta = SlotMetadata {
        state: ImageSlotState::Empty,
        chunk_count: 0,
        base_key: slot as u8,
    };
    let mut buf = [0u8; SlotMetadata::MAX_SERIALIZED_LEN];
    let serialized = meta.serialize_to(&mut buf)?;
    info!("image_file:erase_slot writing empty metadata slot={}", slot);
    wtx.write(&meta_key(slot), serialized).await.map_err(|_| {
        warn!("image_file:erase_slot write meta error slot={}", slot);
    })?;
    info!("image_file:erase_slot wrote empty metadata slot={}", slot);

    // Delete all existing chunk records for this slot.
    for i in 0..old_chunk_count {
        info!(
            "image_file:erase_slot deleting chunk slot={} chunk={}/{}",
            slot, i, old_chunk_count
        );
        wtx.delete(&chunk_key(slot, i)).await.map_err(|_| {
            warn!(
                "image_file:erase_slot delete chunk error slot={} chunk={}",
                slot, i
            );
        })?;
    }
    info!("image_file:erase_slot deleted chunks slot={}", slot);

    info!("image_file:erase_slot committing transaction slot={}", slot);
    wtx.commit().await.map_err(|_| {
        warn!("image_file:erase_slot commit error slot={}", slot);
    })?;
    info!("image_file:erase_slot commit ok slot={}", slot);
    Ok(())
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
    info!(
        "image_file:write_chunk start slot={} chunk={} bytes={}",
        slot,
        chunk_num,
        data.len()
    );
    info!(
        "image_file:write_chunk acquiring write_transaction slot={} chunk={}",
        slot, chunk_num
    );
    let mut wtx = db.write_transaction().await;
    info!(
        "image_file:write_chunk write_transaction acquired slot={} chunk={}",
        slot, chunk_num
    );
    info!(
        "image_file:write_chunk writing chunk slot={} chunk={} bytes={}",
        slot,
        chunk_num,
        data.len()
    );
    wtx.write(&chunk_key(slot, chunk_num), data)
        .await
        .map_err(|_| {
            warn!(
                "image_file:write_chunk write error slot={} chunk={}",
                slot, chunk_num
            );
        })?;
    info!(
        "image_file:write_chunk wrote chunk slot={} chunk={}",
        slot, chunk_num
    );
    info!(
        "image_file:write_chunk committing slot={} chunk={}",
        slot, chunk_num
    );
    wtx.commit().await.map_err(|_| {
        warn!(
            "image_file:write_chunk commit error slot={} chunk={}",
            slot, chunk_num
        );
    })?;
    info!(
        "image_file:write_chunk commit ok slot={} chunk={}",
        slot, chunk_num
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
    info!("image_file:write_slot_metadata start slot={}", slot);
    let mut buf = [0u8; SlotMetadata::MAX_SERIALIZED_LEN];
    let serialized = meta.serialize_to(&mut buf)?;

    info!(
        "image_file:write_slot_metadata acquiring write_transaction slot={}",
        slot
    );
    let mut wtx = db.write_transaction().await;
    info!(
        "image_file:write_slot_metadata write_transaction acquired slot={}",
        slot
    );
    info!(
        "image_file:write_slot_metadata writing metadata slot={}",
        slot
    );
    wtx.write(&meta_key(slot), serialized).await.map_err(|_| {
        warn!("image_file:write_slot_metadata write error slot={}", slot);
    })?;
    info!(
        "image_file:write_slot_metadata wrote metadata slot={}",
        slot
    );
    info!("image_file:write_slot_metadata committing slot={}", slot);
    wtx.commit().await.map_err(|_| {
        warn!("image_file:write_slot_metadata commit error slot={}", slot);
    })?;
    info!("image_file:write_slot_metadata commit ok slot={}", slot);
    Ok(())
}
