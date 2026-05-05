// ekv 1.0.0 depends on embassy-sync 0.6.x; use the renamed dep so the
// CriticalSectionRawMutex satisfies ekv's RawMutex bound.
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
        self.flash.erase(start, end).map_err(|_| ())
    }

    async fn read(&mut self, page_id: PageID, offset: usize, data: &mut [u8]) -> Result<(), ()> {
        let addr = self.page_addr(page_id) + offset as u32;
        self.flash.read(addr, data).map_err(|_| ())
    }

    async fn write(&mut self, page_id: PageID, offset: usize, data: &[u8]) -> Result<(), ()> {
        let addr = self.page_addr(page_id) + offset as u32;
        self.flash.write(addr, data).map_err(|_| ())
    }
}

// ── Database type alias ───────────────────────────────────────────────────────

/// Convenience alias for the ekv database used throughout the storage module.
pub type EkvDatabase = Database<EkvFlash<'static>, CriticalSectionRawMutex>;

// ── Key layout ────────────────────────────────────────────────────────────────
//
// All keys are short byte arrays to stay within EKV_MAX_KEY_SIZE (8 bytes).
//
//   0x01          → active slot index (value: 1 byte: slot as u8)
//   0x02 <slot>   → slot metadata     (value: postcard-encoded SlotMetadata)
//   0x03 <slot> <chunk_hi> <chunk_lo> → image chunk data (value: raw bytes)

/// Key for the active-slot index record.
pub const KEY_ACTIVE_SLOT: &[u8] = &[0x01];

/// Key for the metadata record of `slot`.
#[inline]
pub fn meta_key(slot: usize) -> [u8; 2] {
    [0x02, slot as u8]
}

/// Key for chunk number `chunk_num` of `slot`.
/// `chunk_num` is encoded big-endian so keys sort in chunk order.
#[inline]
pub fn chunk_key(slot: usize, chunk_num: u16) -> [u8; 4] {
    let [hi, lo] = chunk_num.to_be_bytes();
    [0x03, slot as u8, hi, lo]
}
