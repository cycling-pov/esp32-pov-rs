use core::ops::Range;

use crc32fast::Hasher as Crc32Hasher;
use defmt::{info, warn};
use embedded_storage_async::nor_flash::{
    NorFlash as AsyncNorFlash, ReadNorFlash as AsyncReadNorFlash,
};
use sequential_storage::{
    cache::NoCache,
    queue::{QueueConfig, QueueStorage},
};

use super::{AsyncFlash, CHUNK_SIZE};

/// Manages a single image-file slot backed by raw NOR-flash.
///
/// Images are written as a flat byte stream at the start of the partition
/// via [`write_at_offset`] and verified by [`verify_crc`] before being
/// committed.  A two-phase commit is still performed at the config layer:
///
///   1. `config.set_slot_state(slot, Writing, buf)`
///   2. `storage.erase_for_streaming(flash)`
///   3. `storage.write_at_offset(flash, offset, chunk)` — for every chunk
///   4. `storage.verify_crc(flash, total_bytes, expected_crc32, buf)` → Ok
///   5. `config.set_slot_state(slot, Valid { .. }, buf)`
///
/// On boot, a slot still in `Writing` state is treated as `Empty` (power-loss safe).
///
/// Flash is not owned; callers pass `&mut AsyncFlash<'_>` for each operation,
/// allowing a single flash instance to be shared across multiple stores.
pub struct ImageFileStore {
    slot: usize,
    flash_range: Range<u32>,
}

impl ImageFileStore {
    pub fn new(slot: usize, flash_range: Range<u32>) -> Self {
        Self { slot, flash_range }
    }

    fn flash_range(&self) -> Range<u32> {
        self.flash_range.clone()
    }

    // -------------------------------------------------------------------------
    // Streaming write API (replaces write_all / read_all)
    // -------------------------------------------------------------------------

    /// Erase the entire slot partition in preparation for streaming writes.
    ///
    /// Must be called before any [`write_at_offset`] calls for a new image.
    pub async fn erase_for_streaming(&mut self, flash: &mut AsyncFlash<'_>) -> Result<(), ()> {
        info!("image_file:erase_for_streaming slot={}", self.slot);
        let result = flash
            .erase(self.flash_range.start, self.flash_range.end)
            .await
            .map_err(|_| ());
        if result.is_err() {
            warn!("image_file:erase_for_streaming failed slot={}", self.slot);
        }
        result
    }

    /// Write `data` at `offset` bytes from the start of this slot's partition.
    ///
    /// Both `offset` and `data.len()` must be multiples of the flash word size
    /// (4 bytes).  If `data.len()` is not a multiple of 4, the tail is padded
    /// with zero bytes automatically.
    pub async fn write_at_offset(
        &mut self,
        flash: &mut AsyncFlash<'_>,
        offset: u32,
        data: &[u8],
    ) -> Result<(), ()> {
        let addr = self.flash_range.start + offset;
        let aligned_len = data.len() & !3;

        if aligned_len == data.len() {
            // Already word-aligned — write directly.
            flash.write(addr, data).await.map_err(|_| ())
        } else {
            // Write the aligned prefix first (if any), then pad the tail.
            if aligned_len > 0 {
                flash
                    .write(addr, &data[..aligned_len])
                    .await
                    .map_err(|_| ())?;
            }
            let mut tail = [0u8; 4];
            tail[..data.len() - aligned_len].copy_from_slice(&data[aligned_len..]);
            flash
                .write(addr + aligned_len as u32, &tail)
                .await
                .map_err(|_| ())
        }
    }

    /// Read `total_bytes` bytes from offset 0 into `out`.
    ///
    /// `out` must be at least `((total_bytes + 3) & !3)` bytes long so that the
    /// last read can be word-aligned.  After the call only `out[..total_bytes]`
    /// contains valid image data.
    pub async fn read_raw(
        &mut self,
        flash: &mut AsyncFlash<'_>,
        total_bytes: u32,
        out: &mut [u8],
    ) -> Result<(), ()> {
        let aligned_total = ((total_bytes as usize) + 3) & !3;
        let mut offset = 0usize;

        while offset < aligned_total {
            let this_read = (aligned_total - offset).min(CHUNK_SIZE);
            let addr = self.flash_range.start + offset as u32;
            flash
                .read(addr, &mut out[offset..offset + this_read])
                .await
                .map_err(|_| ())?;
            offset += this_read;
        }
        Ok(())
    }

    /// Read `total_bytes` bytes from the slot and verify against `expected_crc32`.
    ///
    /// Returns `Ok(())` on a match, `Err(())` on mismatch or read failure.
    /// `read_buf` is used as a scratch buffer and must be at least `CHUNK_SIZE`
    /// bytes long.
    pub async fn verify_crc(
        &mut self,
        flash: &mut AsyncFlash<'_>,
        total_bytes: u32,
        expected_crc32: u32,
        read_buf: &mut [u8],
    ) -> Result<(), ()> {
        let mut hasher = Crc32Hasher::new();
        let mut offset = 0u32;

        while offset < total_bytes {
            let remaining = (total_bytes - offset) as usize;
            // Hash at most CHUNK_SIZE bytes per iteration.
            let to_hash = remaining.min(CHUNK_SIZE);
            // Read length must be word-aligned; pad up to 4 bytes.
            let to_read = (to_hash + 3) & !3;
            // to_read <= CHUNK_SIZE because CHUNK_SIZE is divisible by 4 and
            // to_hash <= CHUNK_SIZE, so to_read <= CHUNK_SIZE.
            let addr = self.flash_range.start + offset;
            flash
                .read(addr, &mut read_buf[..to_read])
                .await
                .map_err(|_| ())?;
            hasher.update(&read_buf[..to_hash]);
            offset += to_hash as u32;
        }

        let actual_crc = hasher.finalize();
        if actual_crc == expected_crc32 {
            Ok(())
        } else {
            warn!(
                "image_file:verify_crc slot={} expected={=u32:#010x} actual={=u32:#010x}",
                self.slot, expected_crc32, actual_crc
            );
            Err(())
        }
    }

    // -------------------------------------------------------------------------
    // Legacy queue-based API (kept for read_slot_data compatibility)
    // -------------------------------------------------------------------------

    /// Erase the slot and push `data` in `CHUNK_SIZE`-byte chunks.
    ///
    /// Returns the number of chunks written.  The caller is responsible for
    /// updating the slot state in `ConfigStore` before and after this call.
    pub async fn write_all(&mut self, flash: &mut AsyncFlash<'_>, data: &[u8]) -> Result<u16, ()> {
        info!(
            "image_file:write_all start slot={} bytes={} chunk_size={}",
            self.slot,
            data.len(),
            CHUNK_SIZE
        );
        let mut storage =
            QueueStorage::new(flash, QueueConfig::new(self.flash_range()), NoCache::new());
        storage.erase_all().await.map_err(|_| ())?;
        let mut count = 0u16;
        for (index, chunk) in data.chunks(CHUNK_SIZE).enumerate() {
            info!(
                "image_file:write_all push slot={} chunk={} bytes={}",
                self.slot,
                index,
                chunk.len()
            );
            storage.push(chunk, false).await.map_err(|_| ())?;
            count += 1;
        }
        info!(
            "image_file:write_all done slot={} chunks={} bytes={}",
            self.slot,
            count,
            data.len()
        );
        Ok(count)
    }

    /// Iterate every stored chunk (oldest → newest) and pass each to `f`.
    ///
    /// `buf` must be at least `CHUNK_SIZE` bytes.  Does not consume/pop entries.
    pub async fn read_all<F>(
        &mut self,
        flash: &mut AsyncFlash<'_>,
        buf: &mut [u8],
        mut f: F,
    ) -> Result<(), ()>
    where
        F: FnMut(&[u8]),
    {
        info!("image_file:read_all start slot={}", self.slot);
        let mut storage =
            QueueStorage::new(flash, QueueConfig::new(self.flash_range()), NoCache::new());
        let mut iter = storage.iter().await.map_err(|_| ())?;
        let mut count = 0usize;
        let mut total = 0usize;
        while let Some(entry) = iter.next(buf).await.map_err(|_| ())? {
            info!(
                "image_file:read_all entry slot={} chunk={} bytes={}",
                self.slot,
                count,
                entry.len()
            );
            count += 1;
            total += entry.len();
            f(&entry);
        }
        info!(
            "image_file:read_all done slot={} chunks={} bytes={}",
            self.slot, count, total
        );
        Ok(())
    }

    /// Erase all data in this slot's flash region.
    pub async fn erase(&mut self, flash: &mut AsyncFlash<'_>) -> Result<(), ()> {
        info!("image_file:erase slot={}", self.slot);
        let mut storage =
            QueueStorage::new(flash, QueueConfig::new(self.flash_range()), NoCache::new());
        let result = storage.erase_all().await.map_err(|_| ());
        if result.is_err() {
            warn!("image_file:erase failed slot={}", self.slot);
        }
        result
    }
}
