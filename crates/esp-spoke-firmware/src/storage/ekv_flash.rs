// ekv 1.0.0 depends on embassy-sync 0.6.x; use the renamed dep so the
// CriticalSectionRawMutex satisfies ekv's RawMutex bound.
use defmt::{info, warn};
use ekv::{Database, flash::PageID};
use embassy_sync_06::blocking_mutex::raw::CriticalSectionRawMutex;
use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};
use esp_storage::FlashStorage;

// ── Flash adapter ─────────────────────────────────────────────────────────────

/// Page size as configured via EKV_PAGE_SIZE env var.
/// Must match the ESP32 sector erase granularity (4096 bytes).
pub const EKV_PAGE_SIZE: usize = ekv::config::PAGE_SIZE;

/// ESP32 NOR-flash adapter implementing the ekv `Flash` trait.
///
/// Wraps a blocking `FlashStorage` and maps page IDs to absolute flash
/// addresses within the configured partition.
pub struct EkvFlash<'d> {
    flash: FlashStorage<'d>,
    /// Absolute byte address of the first page of this partition.
    partition_start: u32,
    /// Total number of ekv pages in this partition.
    num_pages: usize,
}

impl<'d> EkvFlash<'d> {
    pub fn new(flash: FlashStorage<'d>, partition_start: u32, partition_len: u32) -> Self {
        let num_pages = partition_len as usize / EKV_PAGE_SIZE;
        Self {
            flash,
            partition_start,
            num_pages,
        }
    }

    #[inline]
    fn page_addr(&self, page_id: PageID) -> u32 {
        self.partition_start + page_id.index() as u32 * EKV_PAGE_SIZE as u32
    }
}

impl<'d> ekv::flash::Flash for EkvFlash<'d> {
    type Error = ();

    fn page_count(&self) -> usize {
        self.num_pages
    }

    async fn erase(&mut self, page_id: PageID) -> Result<(), ()> {
        let start = self.page_addr(page_id);
        let end = start + EKV_PAGE_SIZE as u32;
        info!(
            "ekv_flash:erase start page={} addr={=u32:#010x}..{=u32:#010x}",
            page_id.index(),
            start,
            end
        );
        match self.flash.erase(start, end) {
            Ok(()) => {
                info!("ekv_flash:erase ok page={}", page_id.index());
                Ok(())
            }
            Err(e) => {
                warn!(
                    "ekv_flash:erase error page={} addr={=u32:#010x}..{=u32:#010x}: {:?}",
                    page_id.index(),
                    start,
                    end,
                    defmt::Debug2Format(&e)
                );
                Err(())
            }
        }
    }

    async fn read(&mut self, page_id: PageID, offset: usize, data: &mut [u8]) -> Result<(), ()> {
        let addr = self.page_addr(page_id) + offset as u32;
        self.flash.read(addr, data).map_err(|e| {
            warn!(
                "ekv_flash:read error page={} offset={} addr={=u32:#010x} len={}: {:?}",
                page_id.index(),
                offset,
                addr,
                data.len(),
                defmt::Debug2Format(&e)
            );
        })
    }

    async fn write(&mut self, page_id: PageID, offset: usize, data: &[u8]) -> Result<(), ()> {
        let addr = self.page_addr(page_id) + offset as u32;
        // FlashStorage writes must be word-aligned; EKV may emit short values.
        // Use read-modify-write for unaligned writes.
        const WORD_SIZE: usize = 4;
        let addr_usize = addr as usize;

        info!(
            "ekv_flash:write start page={} offset={} len={} addr={=u32:#010x}",
            page_id.index(),
            offset,
            data.len(),
            addr
        );

        if addr_usize.is_multiple_of(WORD_SIZE) && data.len().is_multiple_of(WORD_SIZE) {
            info!(
                "ekv_flash:write direct path page={} addr={=u32:#010x} len={}",
                page_id.index(),
                addr,
                data.len()
            );
            return match self.flash.write(addr, data) {
                Ok(()) => {
                    info!("ekv_flash:write direct ok page={}", page_id.index());
                    Ok(())
                }
                Err(e) => {
                    warn!(
                        "ekv_flash:write direct error page={} addr={=u32:#010x} len={}: {:?}",
                        page_id.index(),
                        addr,
                        data.len(),
                        defmt::Debug2Format(&e)
                    );
                    Err(())
                }
            };
        }

        let start = addr_usize;
        let end = start + data.len();
        let aligned_start = start & !(WORD_SIZE - 1);
        let aligned_end = (end + (WORD_SIZE - 1)) & !(WORD_SIZE - 1);

        info!(
            "ekv_flash:write rmw path page={} addr={=u32:#010x} len={} aligned={=u32:#010x}..{=u32:#010x}",
            page_id.index(),
            addr,
            data.len(),
            aligned_start as u32,
            aligned_end as u32
        );

        let mut src_index = 0usize;
        let mut word_buf = [0u8; WORD_SIZE];
        let verbose_words = data.len() <= 64;

        for word_addr in (aligned_start..aligned_end).step_by(WORD_SIZE) {
            if verbose_words {
                info!(
                    "ekv_flash:write rmw read word addr={=u32:#010x}",
                    word_addr as u32
                );
            }
            self.flash
                .read(word_addr as u32, &mut word_buf)
                .map_err(|e| {
                    warn!(
                        "ekv_flash:write rmw read error page={} addr={=u32:#010x}: {:?}",
                        page_id.index(),
                        word_addr as u32,
                        defmt::Debug2Format(&e)
                    );
                })?;

            for (byte_index, byte) in word_buf.iter_mut().enumerate() {
                let absolute = word_addr + byte_index;
                if absolute >= start && absolute < end {
                    *byte = data[src_index];
                    src_index += 1;
                }
            }

            if verbose_words {
                info!(
                    "ekv_flash:write rmw write word addr={=u32:#010x}",
                    word_addr as u32
                );
            }
            self.flash.write(word_addr as u32, &word_buf).map_err(|e| {
                warn!(
                    "ekv_flash:write rmw write error page={} addr={=u32:#010x}: {:?}",
                    page_id.index(),
                    word_addr as u32,
                    defmt::Debug2Format(&e)
                );
            })?;
        }

        info!("ekv_flash:write rmw ok page={}", page_id.index());
        Ok(())
    }
}

// ── Database type alias ───────────────────────────────────────────────────────

/// Convenience alias for the ekv database used throughout the storage module.
pub type EkvDatabase = Database<EkvFlash<'static>, CriticalSectionRawMutex>;

// ── Key layout ────────────────────────────────────────────────────────────────
//
// All keys are short byte arrays to stay within EKV_MAX_KEY_SIZE (8 bytes).
//
//   0x01                → active image id (value: u32 little-endian)
//   0x02 <id:4>         → image metadata  (value: postcard-encoded SlotMetadata)
//   0x03 <id:4> <chunk:2> → image chunk data (value: raw bytes)
//   0x04                → sensor config   (value: postcard-encoded SensorConfig)
//   0x05                → storage index   (value: postcard-encoded StorageIndex)
//   0x06                → schema version  (value: 1 byte)
//   0x07                → adc monitor sample rate hz (value: u16 little-endian)
//   0x08                → hybrid hall threshold (value: u16 little-endian)
//   0x09                → estimator mode (value: 1 byte)

/// Key for the active-image id record.
pub const KEY_ACTIVE_SLOT: &[u8] = &[0x01];

/// Key for the persisted sensor-config record.
pub const KEY_SENSOR_CONFIG: &[u8] = &[0x04];

/// Key for the persisted storage index record.
pub const KEY_STORAGE_INDEX: &[u8] = &[0x05];

/// Key for storage schema versioning.
pub const KEY_STORAGE_SCHEMA_VERSION: &[u8] = &[0x06];

/// Key for persisted ADC periodic-monitor sample rate in hertz.
pub const KEY_ADC_MONITOR_SAMPLE_RATE_HZ: &[u8] = &[0x07];

/// Key for the hybrid angle estimator hall trigger threshold.
pub const KEY_HYBRID_HALL_TRIGGER_THRESHOLD: &[u8] = &[0x08];

/// Key for the persisted runtime estimator mode.
pub const KEY_ESTIMATOR_MODE: &[u8] = &[0x09];

/// Key for the metadata record of `image_id`.
#[inline]
pub fn meta_key(image_id: u32) -> [u8; 5] {
    let id = image_id.to_le_bytes();
    [0x02, id[0], id[1], id[2], id[3]]
}

/// Key for chunk number `chunk_num` of `image_id`.
/// `chunk_num` is encoded big-endian so keys sort in chunk order.
#[inline]
pub fn chunk_key(image_id: u32, chunk_num: u16) -> [u8; 7] {
    let id = image_id.to_le_bytes();
    let [hi, lo] = chunk_num.to_be_bytes();
    [0x03, id[0], id[1], id[2], id[3], hi, lo]
}
