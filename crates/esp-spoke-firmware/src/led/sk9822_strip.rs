use defmt::{info, warn};
use embassy_futures::select::{Either, select};
use embassy_time::{Duration as EmbassyDuration, Timer};
use esp_hal::{
    Async,
    dma::DmaLoopBuf,
    gpio::{AnyPin, Pin},
    rng::Rng,
    spi::master::SpiDma,
};
use pov_proto::image::{DecodeMode, decode_into_rgb8};
use pov_proto::transfer::{DownloadKind, SpokeCommand};
use smart_leds_trait::RGB8;
use static_cell::StaticCell;

use crate::bitmap::{BitmapStorage, generated_image_storage};
use crate::led::{LedCommand, LedError, LedStrip, LedTimings};
use crate::networking::CompletedDownload;
use crate::storage;
use crate::storage::config::{ImageKind, ImageSlotState};

pub const SK9822_LED_COUNT: usize = 30;

const SK9822_RGB565_DECODE_SCRATCH_BYTES: usize = 1024 * 10;
const DOWNLOADABLE_IMAGE_SLOTS: usize = 2;

const SK9822_MAX_BRIGHTNESS: u8 = 31;
const SK9822_BRIGHTNESS_LIMIT_PERCENT: u8 = 5;
// SK9822 global brightness has 5 bits (0..31). 1/31 ~= 3.2%, 2/31 ~= 6.5%, so
// level 1 is the highest level that does not exceed 5%.
const SK9822_BRIGHTNESS: u8 =
    ((SK9822_MAX_BRIGHTNESS as u16 * SK9822_BRIGHTNESS_LIMIT_PERCENT as u16) / 100) as u8;
const SK9822_START_FRAME_BYTES: usize = 4;

const fn sk9822_end_frame_bytes(led_count: usize) -> usize {
    led_count.div_ceil(16)
}

pub const fn sk9822_frame_size(led_count: usize) -> usize {
    SK9822_START_FRAME_BYTES + (4 * led_count) + sk9822_end_frame_bytes(led_count)
}

pub struct Sk9822Pins<'d> {
    pub clock: AnyPin<'d>,
    pub data: AnyPin<'d>,
}

impl<'d> Sk9822Pins<'d> {
    pub fn new(clock: impl Pin + 'd, data: impl Pin + 'd) -> Self {
        Self {
            clock: clock.degrade(),
            data: data.degrade(),
        }
    }
}

pub struct Sk9822Strip<'d, const LED_COUNT: usize> {
    spi: Option<SpiDma<'d, Async>>,
    dma_buf: Option<DmaLoopBuf>,
    framebuffer: [RGB8; LED_COUNT],
}

impl<'d, const LED_COUNT: usize> Sk9822Strip<'d, LED_COUNT> {
    pub const LED_COUNT: usize = LED_COUNT;
    pub const TIMINGS: LedTimings = LedTimings::SK9822;

    pub fn new(spi: SpiDma<'d, Async>, dma_buf: DmaLoopBuf) -> Self {
        Self {
            spi: Some(spi),
            dma_buf: Some(dma_buf),
            framebuffer: [RGB8::default(); LED_COUNT],
        }
    }

    fn encode_framebuffer(&self, buf: &mut [u8]) {
        buf[..SK9822_START_FRAME_BYTES].fill(0);

        for (index, pixel) in self.framebuffer.iter().copied().enumerate() {
            let offset = SK9822_START_FRAME_BYTES + (index * 4);
            buf[offset] = 0b1110_0000 | SK9822_BRIGHTNESS;
            buf[offset + 1] = pixel.b;
            buf[offset + 2] = pixel.g;
            buf[offset + 3] = pixel.r;
        }

        let end_start = SK9822_START_FRAME_BYTES + (LED_COUNT * 4);
        let end_count = sk9822_end_frame_bytes(LED_COUNT);
        for byte in &mut buf[end_start..end_start + end_count] {
            *byte = 0xFF;
        }
    }
}

impl<const LED_COUNT: usize> LedStrip for Sk9822Strip<'_, LED_COUNT> {
    fn led_count(&self) -> usize {
        self.framebuffer.len()
    }

    fn timings(&self) -> LedTimings {
        Self::TIMINGS
    }

    fn pixels(&self) -> &[RGB8] {
        &self.framebuffer
    }

    fn pixels_mut(&mut self) -> &mut [RGB8] {
        &mut self.framebuffer
    }

    async fn show(&mut self) -> Result<(), LedError> {
        let mut dma_buf = self.dma_buf.take().expect("dma_buf missing");
        self.encode_framebuffer(&mut dma_buf);

        let frame_size = sk9822_frame_size(LED_COUNT);
        let spi = self.spi.take().expect("spi missing");
        let mut transfer = match spi.write(frame_size, dma_buf) {
            Ok(t) => t,
            Err((_, spi, dma_buf)) => {
                self.spi = Some(spi);
                self.dma_buf = Some(dma_buf);
                return Err(LedError::SpiWrite);
            }
        };

        transfer.wait_for_done().await;
        let (spi, dma_buf) = transfer.wait();
        self.spi = Some(spi);
        self.dma_buf = Some(dma_buf);
        Ok(())
    }
}

async fn render_bitmap_index(
    _led_strip: &mut Sk9822Strip<'_, SK9822_LED_COUNT>,
    bitmap_store: &impl BitmapStorage,
    index: usize,
) {
    let _image_bitmap = bitmap_store.bitmap(index).expect("missing bitmap");

    info!("Bitmap rendering not implemented yet");
}

async fn apply_downloaded_image(
    led_strip: &mut Sk9822Strip<'_, SK9822_LED_COUNT>,
    bitmap_store: &mut impl BitmapStorage,
    current_bitmap_index: &mut usize,
    next_download_slot: &mut usize,
    decode_scratch: &mut [u8],
    download: &CompletedDownload,
) {
    let metadata = bitmap_store.metadata();
    let pixel_count = metadata.pixel_count();

    info!(
        "sk9822:download start transfer_id={} kind={:?} bytes={} crc32=0x{:08x}",
        download.transfer_id, download.kind, download.len, download.crc32
    );

    let writable_base = bitmap_store
        .bitmap_count()
        .saturating_sub(DOWNLOADABLE_IMAGE_SLOTS);
    let writable_index = writable_base + (*next_download_slot % DOWNLOADABLE_IMAGE_SLOTS);
    let selected_slot = *next_download_slot % DOWNLOADABLE_IMAGE_SLOTS;
    *next_download_slot = (*next_download_slot + 1) % DOWNLOADABLE_IMAGE_SLOTS;
    info!(
        "sk9822:download select slot={} writable_index={} next_slot={}",
        selected_slot, writable_index, *next_download_slot
    );
    let mut writable = bitmap_store
        .bitmap_mut(writable_index)
        .expect("missing writable image slot");

    let decoded = match decode_into_rgb8(
        download.payload(),
        decode_scratch,
        writable.pixels_mut(),
        DecodeMode::ExactPixels,
    ) {
        Ok(decoded) => decoded,
        Err(err) => {
            info!(
                "ignoring transfer {}: failed to decode framed payload ({:?})",
                download.transfer_id, err
            );
            return;
        }
    };
    info!(
        "decoded transfer {} as {:?} ({} bytes, {} pixels)",
        download.transfer_id, decoded, download.len, pixel_count
    );

    // TODO: Use LED translation utility to map the bitmap to LED strip commands. For now, just set the LEDs to white
    led_strip.fill(smart_leds_trait::RGB8 {
        r: 255,
        g: 255,
        b: 255,
    });
    led_strip
        .show()
        .await
        .expect("failed to show downloaded bitmap on SK9822 strip");
    *current_bitmap_index = writable_index;

    info!(
        "applied downloaded image transfer {} ({} bytes, crc32=0x{:08x})",
        download.transfer_id, download.len, download.crc32
    );

    // Persist the raw compressed payload to flash (two-phase commit).
    let flash_slot = writable_index - writable_base;
    info!(
        "sk9822:flash persist begin transfer_id={} slot={}",
        download.transfer_id, flash_slot
    );
    if storage::set_slot_state(flash_slot, ImageSlotState::Writing)
        .await
        .is_ok()
    {
        match storage::write_slot_data(flash_slot, download.payload()).await {
            Ok(chunk_count) => {
                let state = ImageSlotState::Valid {
                    chunk_count,
                    total_bytes: download.len as u32,
                    kind: ImageKind::Static,
                    encoding: decoded,
                };
                if storage::set_slot_state(flash_slot, state).await.is_err() {
                    warn!("failed to mark flash slot {} as Valid", flash_slot);
                }
                if storage::set_active_slot(flash_slot as u8).await.is_err() {
                    warn!("failed to set active slot to {}", flash_slot);
                }
                info!(
                    "persisted transfer {} to flash slot {}",
                    download.transfer_id, flash_slot
                );
            }
            Err(()) => {
                warn!(
                    "failed to write flash slot {} for transfer {}",
                    flash_slot, download.transfer_id
                );
            }
        }
    } else {
        warn!("failed to mark flash slot {} as Writing", flash_slot);
    }
}

async fn apply_command(
    led_strip: &mut Sk9822Strip<'_, SK9822_LED_COUNT>,
    bitmap_store: &impl BitmapStorage,
    current_bitmap_index: &mut usize,
    randomizing: &mut bool,
    frame: pov_proto::transfer::CommandFrame,
) {
    info!(
        "sk9822:command received transfer_id={} command={:?}",
        frame.transfer_id, frame.command
    );

    match frame.command {
        SpokeCommand::DisplayOff => {
            *randomizing = false;
            led_strip.clear();
            led_strip
                .show()
                .await
                .expect("failed to clear SK9822 strip");
            info!(
                "applied DisplayOff command from transfer {}",
                frame.transfer_id
            );
        }
        SpokeCommand::NextImage => {
            *randomizing = false;
            let bitmap_count = bitmap_store.bitmap_count();
            if bitmap_count == 0 {
                info!(
                    "ignoring NextImage command from transfer {}: no images",
                    frame.transfer_id
                );
                return;
            }

            *current_bitmap_index = (*current_bitmap_index + 1) % bitmap_count;
            render_bitmap_index(led_strip, bitmap_store, *current_bitmap_index).await;

            // Persist the active slot when navigating to a downloaded image.
            let writable_base = bitmap_count.saturating_sub(DOWNLOADABLE_IMAGE_SLOTS);
            if *current_bitmap_index >= writable_base {
                let flash_slot = *current_bitmap_index - writable_base;
                if storage::set_active_slot(flash_slot as u8).await.is_err() {
                    warn!(
                        "failed to persist active slot {} after NextImage",
                        flash_slot
                    );
                }
            }

            info!(
                "applied NextImage command from transfer {}: new_index={}",
                frame.transfer_id, *current_bitmap_index
            );
        }
        SpokeCommand::RandomizeDisplay => {
            *randomizing = true;
            info!(
                "applied RandomizeDisplay command from transfer {}",
                frame.transfer_id
            );
        }
    }
}

#[embassy_executor::task]
pub async fn sk9822_strip_task(mut led_strip: Sk9822Strip<'static, SK9822_LED_COUNT>) -> ! {
    info!(
        "SK9822 strip ready: leds={}, timings={:?}",
        led_strip.led_count(),
        led_strip.timings()
    );

    static DECODE_SCRATCH: StaticCell<[u8; SK9822_RGB565_DECODE_SCRATCH_BYTES]> = StaticCell::new();
    let decode_scratch = DECODE_SCRATCH.init([0; SK9822_RGB565_DECODE_SCRATCH_BYTES]);

    let mut bitmap_store = generated_image_storage();
    let mut current_bitmap_index = 0usize;
    let mut next_download_slot = 0usize;
    let mut randomizing = false;
    let rng = Rng::new();

    // Boot restore: reload any valid flash images into the in-memory writable slots.
    let writable_base = bitmap_store
        .bitmap_count()
        .saturating_sub(DOWNLOADABLE_IMAGE_SLOTS);
    let mut loaded_slots = [false; DOWNLOADABLE_IMAGE_SLOTS];
    for slot in 0..DOWNLOADABLE_IMAGE_SLOTS {
        let state = storage::get_slot_state(slot).await;
        info!("sk9822:boot slot={} state={:?}", slot, state);
        if let ImageSlotState::Valid { encoding: _, .. } = state {
            let read_result = storage::read_slot_data(slot).await;
            let read_ok = read_result.is_ok();
            let img_bytes = read_result.unwrap_or_default();
            info!(
                "sk9822:boot slot={} read_ok={} bytes={}",
                slot,
                read_ok,
                img_bytes.len()
            );
            if read_ok {
                if let Ok(mut writable) = bitmap_store.bitmap_mut(writable_base + slot) {
                    match decode_into_rgb8(
                        &img_bytes,
                        decode_scratch,
                        writable.pixels_mut(),
                        DecodeMode::ExactPixels,
                    ) {
                        Ok(_) => {
                            loaded_slots[slot] = true;
                            info!("restored flash slot {} on boot", slot);
                        }
                        Err(err) => {
                            info!("failed to decode flash slot {} on boot: {:?}", slot, err);
                        }
                    }
                }
            }
        }
    }

    // Restore the active bitmap index from the config store.
    if let Some(active_slot) = storage::get_active_slot().await {
        let slot = active_slot as usize;
        info!(
            "sk9822:boot active_slot={} loaded={}",
            slot,
            slot < DOWNLOADABLE_IMAGE_SLOTS && loaded_slots.get(slot).copied().unwrap_or(false)
        );
        if slot < DOWNLOADABLE_IMAGE_SLOTS && loaded_slots[slot] {
            current_bitmap_index = writable_base + slot;
        }
    } else {
        info!("sk9822:boot no persisted active slot");
    }

    render_bitmap_index(&mut led_strip, &*bitmap_store, current_bitmap_index).await;
    info!(
        "rendered bitmap at startup (index={})",
        current_bitmap_index
    );

    loop {
        let led_cmd = if randomizing {
            let delay = EmbassyDuration::from_millis(10);
            match select(super::LED_COMMAND_CHANNEL.receive(), Timer::after(delay)).await {
                Either::First(cmd) => Some(cmd),
                Either::Second(_) => {
                    led_strip.randomize(&rng);
                    led_strip
                        .show()
                        .await
                        .expect("failed to show randomized SK9822 strip");
                    None
                }
            }
        } else {
            Some(super::LED_COMMAND_CHANNEL.receive().await)
        };

        let Some(led_cmd) = led_cmd else { continue };
        randomizing = false;

        match led_cmd {
            LedCommand::Frame(frame) => {
                info!(
                    "sk9822:loop handling frame transfer_id={} command={:?}",
                    frame.transfer_id, frame.command
                );
                apply_command(
                    &mut led_strip,
                    &*bitmap_store,
                    &mut current_bitmap_index,
                    &mut randomizing,
                    frame,
                )
                .await;
            }
            LedCommand::Download(download) => match download.kind {
                DownloadKind::DisplayImage => {
                    info!(
                        "sk9822:loop handling display download transfer_id={} bytes={}",
                        download.transfer_id, download.len
                    );
                    apply_downloaded_image(
                        &mut led_strip,
                        &mut *bitmap_store,
                        &mut current_bitmap_index,
                        &mut next_download_slot,
                        decode_scratch,
                        &download,
                    )
                    .await
                }
                DownloadKind::OtaImage | DownloadKind::Video => {
                    info!(
                        "ignoring unsupported download kind on SK9822 target: kind={:?} transfer_id={} bytes={}",
                        download.kind, download.transfer_id, download.len
                    );
                }
            },
        }
    }
}
