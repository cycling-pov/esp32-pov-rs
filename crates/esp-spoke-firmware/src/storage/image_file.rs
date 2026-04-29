use defmt::{info, warn};
use sequential_storage::{
    cache::NoCache,
    queue::{QueueConfig, QueueStorage},
};

use super::{AsyncFlash, CHUNK_SIZE, IMG0_FLASH_RANGE, IMG1_FLASH_RANGE};

/// Manages a single image-file slot backed by a sequential-storage queue.
///
/// Each image is split into at most `CHUNK_SIZE`-byte queue entries.
/// Use a two-phase commit around writes:
///   1. `config.set_slot_state(slot, Writing, buf)`
///   2. `image_file.write_all(flash, data)`
///   3. `config.set_slot_state(slot, Valid { .. }, buf)`
///
/// On boot, a slot still in `Writing` state is treated as `Empty` (power-loss safe).
///
/// Flash is not owned; callers pass `&mut AsyncFlash<'_>` for each operation,
/// allowing a single flash instance to be shared across multiple stores.
pub struct ImageFileStore {
    slot: usize,
}

impl ImageFileStore {
    pub fn new(slot: usize) -> Self {
        Self { slot }
    }

    fn flash_range(&self) -> core::ops::Range<u32> {
        if self.slot == 0 {
            IMG0_FLASH_RANGE
        } else {
            IMG1_FLASH_RANGE
        }
    }

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
