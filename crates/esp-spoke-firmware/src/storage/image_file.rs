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
///   2. `image_file.write_all(data, buf)`
///   3. `config.set_slot_state(slot, Valid { .. }, buf)`
///
/// On boot, a slot still in `Writing` state is treated as `Empty` (power-loss safe).
pub struct ImageFileStore<'d> {
    storage: QueueStorage<AsyncFlash<'d>, NoCache>,
}

impl<'d> ImageFileStore<'d> {
    pub fn new(slot: usize, flash: AsyncFlash<'d>) -> Self {
        let range = if slot == 0 {
            IMG0_FLASH_RANGE
        } else {
            IMG1_FLASH_RANGE
        };
        Self {
            storage: QueueStorage::new(flash, QueueConfig::new(range), NoCache::new()),
        }
    }

    /// Erase the slot and push `data` in `CHUNK_SIZE`-byte chunks.
    ///
    /// Returns the number of chunks written.  The caller is responsible for
    /// updating the slot state in `ConfigStore` before and after this call.
    pub async fn write_all(&mut self, data: &[u8]) -> Result<u16, ()> {
        self.storage.erase_all().await.map_err(|_| ())?;
        let mut count = 0u16;
        for chunk in data.chunks(CHUNK_SIZE) {
            self.storage.push(chunk, false).await.map_err(|_| ())?;
            count += 1;
        }
        Ok(count)
    }

    /// Iterate every stored chunk (oldest → newest) and pass each to `f`.
    ///
    /// `buf` must be at least `CHUNK_SIZE` bytes.  Does not consume/pop entries.
    pub async fn read_all<F>(&mut self, buf: &mut [u8], mut f: F) -> Result<(), ()>
    where
        F: FnMut(&[u8]),
    {
        let mut iter = self.storage.iter().await.map_err(|_| ())?;
        while let Some(entry) = iter.next(buf).await.map_err(|_| ())? {
            f(&entry);
        }
        Ok(())
    }

    /// Erase all data in this slot's flash region.
    pub async fn erase(&mut self) -> Result<(), ()> {
        self.storage.erase_all().await.map_err(|_| ())
    }
}
